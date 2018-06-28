use lazy_static::{__lazy_static_create, __lazy_static_internal, lazy_static};
#[allow(unused_imports)]
use log::{debug, error, info, log, trace, warn};
use std::{
    collections::BTreeMap,
    fs::File,
    io::Read,
    net::{SocketAddr, ToSocketAddrs},
    str::FromStr,
};
use toml::{
    self,
    Value::{Array, Table},
};

lazy_static! {
    static ref CONF: Conf = load_conf();
}

#[derive(Default, Debug)]
struct Conf {
    server: Option<toml::Value>,
    client: Option<toml::Value>,
}

crate fn init() {
    ::lazy_static::initialize(&CONF);
}

fn load_conf() -> Conf {
    trace!("Loading configuration");
    let mut result = Conf::default();
    result.load_server_conf();
    result.load_client_conf();
    trace!("Configuration result: {:?}", result);
    result
}

fn load_from_home(path: &str) -> Option<toml::Value> {
    let mut path = path.to_string();
    if path.starts_with("~") {
        let home = ::std::env::home_dir();
        if home.is_none() {
            return None;
        }
        let home = home.unwrap();
        let home = home.to_str().unwrap().to_string();
        path = path.replacen('~', &home, 1);
    }
    let file = File::open(&path);
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
        let path = crate::arg::matches().value_of("client config");
        if path.is_none() {
            return;
        }
        let path = path.unwrap();
        self.client = load_from_home(path);
    }

    fn load_server_conf(&mut self) {
        let path = crate::arg::matches().value_of("server config");
        if path.is_none() {
            return;
        }
        let path = path.unwrap();
        let mut result = load_from_home(path);

        let mut name_ticker: u64 = 0;

        if let Some(table) = result.as_mut() {
            if let Some(clients) = table.get_mut("clients") {
                if let Some(clients) = clients.as_array_mut() {
                    for client in clients {
                        if let Some(client) = client.as_table_mut() {
                            if !client.contains_key("name") {
                                client.insert(
                                    "name".to_string(),
                                    ::toml::Value::String(format!("generated-client-name-{}", name_ticker)),
                                );
                                name_ticker += 1;
                            } else {
                                if let Some(name) = client.get("name").unwrap().as_str() {
                                    if name.starts_with("generated-client-name-") {
                                        warn!("Warning!!! You are using a statically set client name that starts with 'generated-client-name-'. This is not recommended.");
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        self.server = result;
    }
}

crate fn default_server_knock() -> Option<Vec<u8>> {
    Some(
        ::data_encoding::BASE32_NOPAD
            .decode(CONF.server.as_ref()?.as_table()?.get("knock")?.as_str()?.as_bytes())
            .ok()?
            .to_vec(),
    )
}

crate fn default_client_knock() -> Option<Vec<u8>> {
    Some(
        ::data_encoding::BASE32_NOPAD
            .decode(CONF.client.as_ref()?.as_table()?.get("knock")?.as_str()?.as_bytes())
            .ok()?
            .to_vec(),
    )
}

crate fn peer_knock(peer: &str) -> Option<Vec<u8>> {
    let table = match crate::arg::mode().as_str() {
        "server" => client(peer),
        "client" | "copy" => server(peer),
        _ => None,
    };
    Some(::data_encoding::BASE32_NOPAD.decode(table?.get("knock")?.as_str()?.as_bytes()).ok()?)
}

crate fn default_knock() -> Option<Vec<u8>> {
    match crate::arg::mode().as_str() {
        "server" => default_server_knock(),
        "client" | "copy" => default_client_knock(),
        _ => None,
    }
}

crate fn default_asymmetric_key() -> Option<Vec<u8>> {
    match crate::arg::mode().as_str() {
        "server" | "reexec" => ::data_encoding::BASE32_NOPAD
            .decode(CONF.server.as_ref()?.as_table()?.get("privkey")?.as_str()?.as_bytes())
            .ok(),
        "client" | "copy" => ::data_encoding::BASE32_NOPAD
            .decode(CONF.client.as_ref()?.as_table()?.get("privkey")?.as_str()?.as_bytes())
            .ok(),
        _ => None,
    }
}

crate fn peer_asymmetric_key(peer: &str) -> Option<Vec<u8>> {
    match crate::arg::mode().as_str() {
        "server" | "reexec" => ::data_encoding::BASE32_NOPAD
            .decode(client(peer)?.get("privkey")?.as_str()?.as_bytes())
            .ok(),
        "client" | "copy" => ::data_encoding::BASE32_NOPAD
            .decode(server(peer)?.get("privkey")?.as_str()?.as_bytes())
            .ok(),
        _ => None,
    }
}

crate fn peer_static_key(peer: &str) -> Option<Vec<u8>> {
    match crate::arg::mode().as_str() {
        "server" | "reexec" => ::data_encoding::BASE32_NOPAD.decode(client(peer)?.get("psk")?.as_str()?.as_bytes()).ok(),
        "client" | "copy" => ::data_encoding::BASE32_NOPAD.decode(server(peer)?.get("psk")?.as_str()?.as_bytes()).ok(),
        _ => None,
    }
}

crate fn peer_public_key(peer: &str) -> Option<Vec<u8>> {
    match crate::arg::mode().as_str() {
        "server" | "reexec" => ::data_encoding::BASE32_NOPAD
            .decode(client(peer)?.get("pubkey")?.as_str()?.as_bytes())
            .ok(),
        "client" | "copy" => ::data_encoding::BASE32_NOPAD
            .decode(server(peer)?.get("pubkey")?.as_str()?.as_bytes())
            .ok(),
        _ => None,
    }
}

crate fn default_static_key() -> Option<Vec<u8>> {
    match crate::arg::mode().as_str() {
        "server" | "reexec" => ::data_encoding::BASE32_NOPAD
            .decode(CONF.server.as_ref()?.as_table()?.get("psk")?.as_str()?.as_bytes())
            .ok(),
        "client" | "copy" => ::data_encoding::BASE32_NOPAD
            .decode(CONF.client.as_ref()?.as_table()?.get("psk")?.as_str()?.as_bytes())
            .ok(),
        _ => None,
    }
}

crate fn asymmetric_key(peer: Option<&str>) -> Option<Vec<u8>> {
    if let Some(peer) = peer {
        if let Some(key) = peer_asymmetric_key(peer) {
            return Some(key);
        }
    }
    if let Some(key) = default_asymmetric_key() {
        return Some(key);
    }
    None
}

crate fn static_key(peer: Option<&str>) -> Option<Vec<u8>> {
    if let Some(peer) = peer {
        if let Some(key) = peer_static_key(peer) {
            return Some(key);
        }
    }
    if let Some(key) = default_static_key() {
        return Some(key);
    }
    None
}

crate fn public_key(peer: Option<&str>) -> Option<Vec<u8>> {
    if let Some(peer) = peer {
        if let Some(key) = peer_public_key(peer) {
            return Some(key);
        }
    }
    None
}

crate fn get_setuser(peer: &str) -> Option<String> {
    Some(client(peer)?.get("setuser")?.as_str()?.to_string())
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

crate fn clients() -> BTreeMap<String, BTreeMap<String, toml::Value>> {
    match &CONF.server {
        Some(Table(table)) => {
            let clients = table.get("clients");
            match clients {
                Some(Array(clients)) => {
                    let mut result = BTreeMap::new();
                    for client in clients {
                        if !client.is_table() {
                            continue;
                        }
                        let client = client.as_table().unwrap();
                        let name = client.get("name");
                        if name.is_none() {
                            continue;
                        }
                        let name = name.unwrap();
                        if !name.is_str() {
                            continue;
                        }
                        let name = name.as_str().unwrap().to_string();
                        if result.contains_key(&name) {
                            warn!("Duplicate configuration file entry detected");
                            continue;
                        }
                        result.insert(name, client.clone());
                    }
                    return result;
                }
                _ => (),
            }
        }
        _ => (),
    }
    BTreeMap::new()
}

crate fn servers() -> BTreeMap<String, BTreeMap<String, toml::Value>> {
    match &CONF.client {
        Some(Table(table)) => {
            let servers = table.get("servers");
            match servers {
                Some(Array(servers)) => {
                    let mut result = BTreeMap::new();
                    for server in servers {
                        if !server.is_table() {
                            continue;
                        }
                        let server = server.as_table().unwrap();
                        let name = server.get("name");
                        if name.is_none() {
                            continue;
                        }
                        let name = name.unwrap();
                        if !name.is_str() {
                            continue;
                        }
                        let name = name.as_str().unwrap().to_string();
                        if result.contains_key(&name) {
                            warn!("Duplicate configuration file entry detected");
                            continue;
                        }
                        result.insert(name, server.clone());
                    }
                    return result;
                }
                _ => (),
            }
        }
        _ => (),
    }
    BTreeMap::new()
}

crate fn client(client: &str) -> Option<BTreeMap<String, toml::Value>> {
    clients().get(client).map(|x| x.clone())
}

crate fn server(server: &str) -> Option<BTreeMap<String, toml::Value>> {
    servers().get(server).map(|x| x.clone())
}

crate fn pubkey_for_client(client: &str) -> Option<Vec<u8>> {
    let clients = clients();
    let client = clients.get(client)?;
    let key = client.get("pubkey")?;
    let key = key.as_str()?;
    let key = ::data_encoding::BASE32_NOPAD.decode(key.as_bytes()).ok()?;
    Some(key.to_vec())
}

crate fn client_names() -> Vec<String> {
    clients().keys().map(|x| x.to_string()).collect()
}

crate fn host_for_dest(dest: &str) -> String {
    let table = server(dest);
    if table.is_none() {
        return host_part(dest).to_string();
    }
    let table = table.unwrap();
    let entry = table.get("host");
    if entry.is_none() {
        return host_part(dest).to_string();
    }
    let entry = entry.unwrap();
    if !entry.is_str() {
        warn!("Host value in config is not a string?");
        return host_part(dest).to_string();
    }
    entry.as_str().unwrap().to_string()
}

fn conf_port_for_dest(dest: &str) -> Option<u16> {
    if let Some(table) = server(dest) {
        let port = table.get("port");
        if port.is_none() {
            return None;
        }
        let port = port.unwrap();
        if port.is_integer() {
            return Some(port.as_integer().unwrap() as u16);
        }
        if port.is_str() {
            let port = port.as_str().unwrap().parse();
            if port.is_err() {
                warn!("Invalid port value in config");
                return None;
            }
            return Some(port.unwrap());
        }
        warn!("Invalid port value in config.");
        return None;
    }
    None
}

crate fn port_for_dest(dest: &str) -> u16 {
    let port = conf_port_for_dest(dest);
    if port.is_some() {
        return port.unwrap();
    }
    let port = port_part(dest);
    if port.is_some() {
        return port.unwrap();
    }
    return 2600;
}

crate fn canonicalize_destination(dest: &str) -> String {
    let table = server(dest);
    if table.is_none() {
        let port = port_part(dest);
        let host = host_part(dest);
        return format!("{}:{}", host, port.unwrap_or(2600));
    }
    let host = host_for_dest(dest);
    let port = port_for_dest(dest);
    format!("{}:{}", host, port)
}

crate fn locate_destination(dest: &str) -> Vec<SocketAddr> {
    let host = host_for_dest(dest);
    let port = port_for_dest(dest);
    let result = (host.as_str(), port).to_socket_addrs();
    if result.is_err() {
        return Vec::new();
    }
    return result.unwrap().collect();
}

crate fn forced_command(peer: Option<&str>) -> Option<String> {
    serverside_setting(peer, "forced command", "forcedcommand")
}

crate fn multiplexer(peer: Option<&str>) -> Option<String> {
    serverside_setting(peer, "multiplexer", "multiplexer")
}

crate fn serverside_setting(peer: Option<&str>, arg: &str, key: &str) -> Option<String> {
    if crate::arg::matches().occurrences_of(arg) > 0 {
        let setting = crate::arg::matches().value_of(arg);
        if setting.is_some() {
            return Some(setting.unwrap().to_string());
        }
    }
    if peer.is_some() {
        let client = client(peer.unwrap());
        if let Some(client) = client {
            let setting = client.get(key);
            if let Some(setting) = setting {
                if let Some(setting) = setting.as_str() {
                    return Some(setting.to_string());
                }
            }
        }
    }

    if let Some(server) = CONF.server.as_ref() {
        if let Some(server) = server.as_table() {
            if let Some(setting) = server.get(key) {
                if let Some(setting) = setting.as_str() {
                    return Some(setting.to_string());
                }
            }
        }
    }

    crate::arg::matches().value_of(arg).map(|x| x.to_string())
}
