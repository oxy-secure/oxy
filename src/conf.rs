use lazy_static::lazy_static;
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

fn load_conf() -> Conf {
    trace!("Loading configuration");
    let mut result = Conf::default();
    match crate::arg::mode().as_str() {
        "server" | "serve-one" | "reverse-server" => {
            result.load_server_conf();
        }
        "client" | "copy" | "reverse-client" => {
            result.load_client_conf();
        }
        _ => (),
    }
    trace!("Configuration result: {:?}", result);
    result
}

fn decrypt(mut data: Vec<u8>, passphrase: &str) -> Option<String> {
    let salt = data.drain(..32).collect::<Vec<u8>>();
    let nonce = data.drain(..12).collect::<Vec<u8>>();
    let difficulty = <::byteorder::BE as ::byteorder::ByteOrder>::read_u32(&data.drain(..4).collect::<Vec<u8>>());
    let mut key = [0u8; 32];
    ::ring::pbkdf2::derive(&::ring::digest::SHA512, difficulty, &salt, passphrase.as_bytes(), &mut key);
    let key = ::ring::aead::OpeningKey::new(&::ring::aead::AES_256_GCM, &key).unwrap();
    let result = ::ring::aead::open_in_place(&key, &nonce, b"", 0, &mut data);
    if result.is_err() {
        return None;
    }
    let result = ::std::str::from_utf8(result.unwrap());
    if result.is_err() {
        return None;
    }
    Some(result.unwrap().to_string())
}

fn encrypt(data: &[u8], passphrase: &str) -> Vec<u8> {
    let mut rng = ::snow::CryptoResolver::resolve_rng(&::snow::DefaultResolver).unwrap();
    let mut salt = [0u8; 32];
    let mut nonce = [0u8; 12];
    ::snow::types::Random::fill_bytes(&mut *rng, &mut nonce);
    ::snow::types::Random::fill_bytes(&mut *rng, &mut salt);
    let mut difficulty = [0u8; 4];
    <::byteorder::BE as ::byteorder::ByteOrder>::write_u32(&mut difficulty, 10240u32);
    let mut key = [0u8; 32];
    ::ring::pbkdf2::derive(&::ring::digest::SHA512, 10240, &salt, passphrase.as_bytes(), &mut key);
    let key = ::ring::aead::SealingKey::new(&::ring::aead::AES_256_GCM, &key).unwrap();
    let mut encrypt_buf = data.to_vec();
    encrypt_buf.resize(data.len() + 16, 0);
    let encrypted_size = ::ring::aead::seal_in_place(&key, &nonce, b"", &mut encrypt_buf, 16);
    if encrypted_size.is_err() {
        error!("Failed to encrypt config: {:?}", encrypted_size);
        ::std::process::exit(1);
    }
    debug_assert!(encrypted_size.unwrap() == encrypt_buf.len());
    let mut result = salt.to_vec();
    result.extend(&nonce);
    result.extend(&difficulty);
    result.extend(&encrypt_buf);
    result
}

fn read_passphrase(passphrase_for: &str) -> Result<String, String> {
    #[cfg(not(unix))]
    unimplemented!();
    #[cfg(unix)]
    {
        use std::io::Write;
        let mut tty = ::termion::get_tty().map_err(|_| "Failed to acquire TTY")?;
        let mut tty2 = ::termion::get_tty().map_err(|_| "Failed to acquire TTY")?;
        let _ = write!(tty, "Please enter passphrase for {}: ", passphrase_for);
        let passphrase = ::termion::input::TermRead::read_passwd(&mut tty, &mut tty2);
        let _ = write!(tty, "\n");
        if passphrase.is_err() {
            Err("Failed to read passphrase")?;
        }
        let passphrase = passphrase.unwrap();
        if passphrase.is_none() {
            Err("Failed to read passphrase")?;
        }
        Ok(passphrase.unwrap())
    }
}

fn decrypt_config(input_config: toml::Value, file_path: &str) -> Result<(toml::Value, Option<String>), String> {
    let is_encrypted = input_config
        .as_table()
        .ok_or("Failed to interpret toml as table")?
        .contains_key("encrypted");
    if !is_encrypted {
        // It's pretty easy to decrypt a config that isn't encrypted.
        return Ok((input_config, None));
    }
    let table = input_config.as_table().unwrap();
    let encrypted_config = table.get("encrypted").unwrap();
    let encrypted_config = ::data_encoding::BASE32_NOPAD
        .decode(encrypted_config.as_str().ok_or("Encrypted config is not a string?")?.as_bytes())
        .map_err(|_| "Failed to decode encrypted config value.")?;

    let passphrase = read_passphrase(file_path)?;
    let plaintext_config = decrypt(encrypted_config, &passphrase);
    if plaintext_config.is_none() {
        Err("Decryption failed")?;
    }
    let plaintext_config = plaintext_config.unwrap();
    let result_config = toml::Value::from_str(&plaintext_config);
    if result_config.is_err() {
        Err(format!("Failed to parse decrypted config. {:?}", result_config))?;
    }
    Ok((result_config.unwrap(), Some(passphrase)))
}

fn resolve_path(path: &str) -> String {
    let mut path = path.to_string();
    if path.starts_with("~") {
        let home = ::dirs::home_dir();
        if home.is_none() {
            error!("Failed to find home directory");
            ::std::process::exit(1);
        }
        let home = home.unwrap();
        let home = home.to_str().unwrap().to_string();
        path = path.replacen('~', &home, 1);
    }
    path
}

fn toml_from_disk(path: &str) -> Option<toml::Value> {
    let path = resolve_path(path);
    let file = File::open(&path);
    if file.is_err() {
        debug!("No {} config to load.", path);
        return None;
    }
    let mut file = file.unwrap();
    #[cfg(unix)]
    {
        let permissions = file.metadata().unwrap().permissions();
        let permissions_mode = ::std::os::unix::fs::PermissionsExt::mode(&permissions);
        if permissions_mode & 0o077 > 0 {
            error!(
                "File permissions on {} are too loose ({:04o}). Please restrict this file to owner-only access.",
                path,
                permissions_mode & 0o7777
            );
            ::std::process::exit(0);
        }
    }
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
        return None;
    }
    value.ok()
}

fn load_file(path: &str) -> Option<(toml::Value, Option<String>)> {
    let value = toml_from_disk(path)?;
    let value = decrypt_config(value, &path);
    if value.is_err() {
        error!("Failed to decrypt config file.");
        return None;
    }
    debug!("Successfully loaded {:?}", path);
    value.ok()
}

impl Conf {
    fn load_client_conf(&mut self) {
        let path = crate::arg::matches().value_of("config").unwrap_or("~/.config/oxy/client.conf");
        self.client = load_file(path).map(|x| x.0);
    }

    fn load_server_conf(&mut self) {
        let path = crate::arg::matches().value_of("config").unwrap_or("~/.config/oxy/server.conf");
        let mut result = load_file(path).map(|x| x.0);

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
                                        error!("Config file contains statically set client name that starts with 'generated-client-name-'.");
                                        ::std::process::exit(1);
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
        "server" | "serve-one" => client(peer),
        "client" | "copy" => server(peer),
        _ => None,
    };
    Some(::data_encoding::BASE32_NOPAD.decode(table?.get("knock")?.as_str()?.as_bytes()).ok()?)
}

crate fn default_knock() -> Option<Vec<u8>> {
    match crate::arg::mode().as_str() {
        "server" | "serve-one" => default_server_knock(),
        "client" | "copy" => default_client_knock(),
        _ => None,
    }
}

crate fn default_asymmetric_key() -> Option<Vec<u8>> {
    match crate::arg::mode().as_str() {
        "server" | "serve-one" | "reverse-server" => ::data_encoding::BASE32_NOPAD
            .decode(CONF.server.as_ref()?.as_table()?.get("privkey")?.as_str()?.as_bytes())
            .ok(),
        "client" | "copy" | "reverse-client" => ::data_encoding::BASE32_NOPAD
            .decode(CONF.client.as_ref()?.as_table()?.get("privkey")?.as_str()?.as_bytes())
            .ok(),
        _ => None,
    }
}

crate fn peer_asymmetric_key(peer: &str) -> Option<Vec<u8>> {
    match crate::arg::mode().as_str() {
        "server" | "serve-one" | "reverse-server" => ::data_encoding::BASE32_NOPAD
            .decode(client(peer)?.get("privkey")?.as_str()?.as_bytes())
            .ok(),
        "client" | "copy" | "reverse-client" => ::data_encoding::BASE32_NOPAD
            .decode(server(peer)?.get("privkey")?.as_str()?.as_bytes())
            .ok(),
        _ => None,
    }
}

crate fn peer_static_key(peer: &str) -> Option<Vec<u8>> {
    match crate::arg::mode().as_str() {
        "server" | "serve-one" | "reverse-server" => ::data_encoding::BASE32_NOPAD.decode(client(peer)?.get("psk")?.as_str()?.as_bytes()).ok(),
        "client" | "copy" | "reverse-client" => ::data_encoding::BASE32_NOPAD.decode(server(peer)?.get("psk")?.as_str()?.as_bytes()).ok(),
        _ => None,
    }
}

crate fn peer_public_key(peer: &str) -> Option<Vec<u8>> {
    match crate::arg::mode().as_str() {
        "server" | "serve-one" | "reverse-server" => ::data_encoding::BASE32_NOPAD
            .decode(client(peer)?.get("pubkey")?.as_str()?.as_bytes())
            .ok(),
        "client" | "copy" | "reverse-client" => ::data_encoding::BASE32_NOPAD
            .decode(server(peer)?.get("pubkey")?.as_str()?.as_bytes())
            .ok(),
        _ => None,
    }
}

crate fn default_static_key() -> Option<Vec<u8>> {
    match crate::arg::mode().as_str() {
        "server" | "serve-one" | "reverse-server" => ::data_encoding::BASE32_NOPAD
            .decode(CONF.server.as_ref()?.as_table()?.get("psk")?.as_str()?.as_bytes())
            .ok(),
        "client" | "copy" | "reverse-client" => ::data_encoding::BASE32_NOPAD
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

crate fn knock_port_for_dest(dest: &str) -> u16 {
    if let Some(port) = crate::arg::matches().value_of("knock port") {
        if let Ok(port) = port.parse() {
            return port;
        } else {
            error!("Failed to parse provided knock port.");
            ::std::process::exit(1);
        }
    }

    if let Some(table) = server(dest) {
        if let Some(knock_port) = table.get("knock_port") {
            if let Some(knock_port) = knock_port.as_integer() {
                if knock_port < 0 || knock_port > ::std::u16::MAX as i64 {
                    panic!("Invalid knock port in configuration.");
                }
                return knock_port as u16;
            }
        }
    }

    error!("Failed to find knock port for {}", dest);
    ::std::process::exit(1);
}

crate fn tcp_port_for_dest(dest: &str) -> u16 {
    if let Some(port) = crate::arg::matches().value_of("tcp port") {
        if let Ok(port) = port.parse() {
            return port;
        } else {
            error!("Failed to parse provided TCP port");
            ::std::process::exit(1);
        }
    }

    if let Some(table) = server(dest) {
        if let Some(tcp_port) = table.get("tcp_port") {
            if let Some(tcp_port) = tcp_port.as_integer() {
                if tcp_port < 0 || tcp_port > ::std::u16::MAX as i64 {
                    panic!("Invalid TCP port");
                }
                return tcp_port as u16;
            }
        }
    }

    knock_port_for_dest(dest)
}

crate fn server_knock_port() -> u16 {
    if let Some(port) = crate::arg::matches().value_of("knock port") {
        if let Ok(port) = port.parse() {
            return port;
        } else {
            error!("Failed to parse provided knock port");
            ::std::process::exit(1);
        }
    }

    if let Some(table) = &CONF.server {
        if let Some(table) = table.as_table() {
            if let Some(knock_port) = table.get("knock_port") {
                if let Some(knock_port) = knock_port.as_integer() {
                    if knock_port < 0 || knock_port > ::std::u16::MAX as i64 {
                        panic!("Invalid knock port");
                    }
                    return knock_port as u16;
                } else {
                    warn!("Invalid knock port in configuration");
                }
            }
        }
    }

    error!("No knock port specified.");
    ::std::process::exit(1);
}

crate fn server_tcp_port() -> u16 {
    if let Some(port) = crate::arg::matches().value_of("tcp port") {
        if let Ok(port) = port.parse() {
            return port;
        } else {
            error!("Failed to parse provided TCP port");
            ::std::process::exit(1);
        }
    }

    if let Some(table) = &CONF.server {
        if let Some(table) = table.as_table() {
            if let Some(tcp_port) = table.get("tcp_port") {
                if let Some(tcp_port) = tcp_port.as_integer() {
                    if tcp_port < 0 || tcp_port > ::std::u16::MAX as i64 {
                        panic!("Invalid TCP port");
                    }
                    return tcp_port as u16;
                } else {
                    warn!("Invalid TCP port in configuration");
                }
            }
        }
    }

    server_knock_port()
}

crate fn host_for_dest(dest: &str) -> String {
    if let Some(table) = server(dest) {
        if let Some(host) = table.get("host") {
            if let Some(host) = host.as_str() {
                return host.to_string();
            }
        }
    }

    dest.to_string()
}

crate fn canonicalize_destination(dest: &str) -> String {
    let table = server(dest);
    if table.is_none() {
        error!("Unknown destination {:?}", dest);
        ::std::process::exit(1);
    }
    let host = host_for_dest(dest);
    let port = tcp_port_for_dest(dest);
    format!("{}:{}", host, port)
}

crate fn locate_destination(dest: &str) -> Vec<SocketAddr> {
    let host = host_for_dest(dest);
    let port = tcp_port_for_dest(dest);
    let result = (host.as_str(), port).to_socket_addrs();
    if result.is_err() {
        return Vec::new();
    }
    return result.unwrap().collect();
}

crate fn forced_command(peer: Option<&str>) -> Option<String> {
    serverside_setting(peer, "forcedcommand", "forcedcommand")
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

crate fn configure() {
    let subcommand = crate::arg::matches().subcommand_name().unwrap().to_string();
    match subcommand.as_str() {
        "initialize-server" => initialize_server(),
        "encrypt-config" => subcommand_encrypt_config(),
        "decrypt-config" => subcommand_decrypt_config(),
        "learn-server" => subcommand_learn_server(),
        "learn-client" => subcommand_learn_client(),
        "delete-client" => subcommand_delete_client(),
        "delete-server" => subcommand_delete_server(),
        _ => unimplemented!(),
    }
}

fn subcommand_decrypt_config() {
    let config_path = crate::arg::matches().subcommand().1.unwrap().value_of("config");
    if config_path.is_none() {
        error!("No config path specified");
        ::std::process::exit(1);
    }
    let config_path = config_path.unwrap();
    let config = load_file(&config_path).map(|x| x.0);
    if config.is_none() {
        error!("Failed to load config.");
        ::std::process::exit(1);
    }
    save_config(&config_path, config.unwrap(), None);
    info!("Config file decrypted successfully");
}

fn subcommand_encrypt_config() {
    let config_path = crate::arg::matches().subcommand().1.unwrap().value_of("config");
    if config_path.is_none() {
        error!("No config path specified");
        ::std::process::exit(1);
    }
    let config_path = config_path.unwrap();
    let config = load_file(&config_path).unwrap().0;
    let passphrase = read_passphrase(&config_path).unwrap();

    save_config(&config_path, config, Some(passphrase.as_str()));
    info!("Configuration successfully encrypted.");
}

fn encrypt_config(unencrypted_config: ::toml::Value, passphrase: &str) -> ::toml::Value {
    let unencrypted_data = ::toml::to_string(&unencrypted_config).unwrap().into_bytes();
    let encrypted_data = encrypt(&unencrypted_data, passphrase);
    let encrypted_data = ::data_encoding::BASE32_NOPAD.encode(&encrypted_data);
    let mut output_config = ::toml::value::Table::new();
    output_config.insert("encrypted".to_string(), ::toml::value::Value::String(encrypted_data));
    ::toml::Value::Table(output_config)
}

fn random_port() -> u16 {
    let mut rng = ::snow::CryptoResolver::resolve_rng(&::snow::DefaultResolver).unwrap();
    loop {
        let mut buf = [0u8; 2];
        ::snow::types::Random::fill_bytes(&mut *rng, &mut buf);
        let cur: u16 = <::byteorder::NativeEndian as ::byteorder::ByteOrder>::read_u16(&buf);
        if cur > 1024 {
            return cur;
        }
    }
}

fn save_config(path: &str, mut config: ::toml::Value, passphrase: Option<&str>) {
    if passphrase.is_some() {
        config = encrypt_config(config, passphrase.unwrap());
    }

    let path = resolve_path(path);
    let dir = ::std::path::PathBuf::from(&path);
    ::std::fs::create_dir_all(dir.parent().unwrap()).unwrap();

    #[cfg(unix)]
    let file = {
        let mut open_options = ::std::fs::OpenOptions::new();
        open_options.create(true).truncate(true).write(true);
        ::std::os::unix::fs::OpenOptionsExt::mode(&mut open_options, 0o600);
        open_options.open(&path)
    };
    #[cfg(not(unix))]
    let file = ::std::fs::File::create(&path);
    if file.is_err() {
        error!("Error opening {} for writing", path);
        ::std::process::exit(1);
    }
    let mut file = file.unwrap();
    ::std::io::Write::write_all(&mut file, config.to_string().as_bytes()).unwrap();
}

fn initialize_server() {
    let mut dh = ::snow::CryptoResolver::resolve_dh(&::snow::DefaultResolver, &::snow::params::DHChoice::Curve25519).unwrap();
    let mut rng = ::snow::CryptoResolver::resolve_rng(&::snow::DefaultResolver).unwrap();
    ::snow::types::Dh::generate(&mut *dh, &mut *rng);

    let privkey = ::snow::types::Dh::privkey(&*dh);
    let pubkey = ::snow::types::Dh::pubkey(&*dh);
    let mut knock = [0u8; 32];
    ::snow::types::Random::fill_bytes(&mut *rng, &mut knock);

    let tcp_port = crate::arg::matches().subcommand().1.unwrap().value_of("tcp-port").unwrap_or("0");
    let knock_port = crate::arg::matches().subcommand().1.unwrap().value_of("knock-port").unwrap_or("0");
    let tcp_port = tcp_port.parse::<u16>();
    let knock_port = knock_port.parse::<u16>();
    if tcp_port.is_err() {
        error!("Failed to parse TCP port");
        ::std::process::exit(1);
    }
    if knock_port.is_err() {
        error!("Failed to parse knock port");
        ::std::process::exit(1);
    }
    let tcp_port = tcp_port.unwrap();
    let knock_port = knock_port.unwrap();
    let tcp_port = if tcp_port == 0 { random_port() } else { tcp_port };
    let knock_port = if knock_port == 0 { random_port() } else { knock_port };

    let privkey = ::data_encoding::BASE32_NOPAD.encode(privkey);
    let pubkey = ::data_encoding::BASE32_NOPAD.encode(pubkey);
    let knock = ::data_encoding::BASE32_NOPAD.encode(&knock);

    let privkey = ::toml::value::Value::String(privkey);
    let pubkey = ::toml::value::Value::String(pubkey);
    let knock = ::toml::value::Value::String(knock);

    let mut config = ::toml::value::Table::new();
    config.insert("pubkey".to_string(), pubkey);
    config.insert("knock".to_string(), knock);
    config.insert("tcp_port".to_string(), ::toml::value::Value::Integer(tcp_port as i64));
    config.insert("knock_port".to_string(), ::toml::value::Value::Integer(knock_port as i64));

    let display_config = ::toml::to_string(&config).unwrap();

    config.insert("privkey".to_string(), privkey);

    let config_path = crate::arg::matches().subcommand().1.unwrap().value_of("config").unwrap();

    save_config(&config_path, config.into(), None);
    println!("{}", display_config);
    println!(
        "Import this server into a client using this string: {}",
        ::data_encoding::BASE32_NOPAD.encode(&display_config.as_bytes())
    );
}

fn drop_server_named(config: &mut ::toml::Value, name: &str) {
    if config.as_table().unwrap().get("servers").is_none() {
        return;
    }
    let server_count = config.as_table().unwrap().get("servers").unwrap().as_array().unwrap().len();
    for i in 0..server_count {
        if config
            .as_table()
            .unwrap()
            .get("servers")
            .unwrap()
            .as_array()
            .unwrap()
            .get(i)
            .unwrap()
            .get("name")
            .map(|x| x.as_str().unwrap())
            == Some(name)
        {
            config
                .as_table_mut()
                .unwrap()
                .get_mut("servers")
                .unwrap()
                .as_array_mut()
                .unwrap()
                .remove(i);
            drop_server_named(config, name);
            return;
        }
    }
}

fn load_import_config() -> ::toml::Value {
    let subcommand_matches = crate::arg::matches().subcommand().1.unwrap();
    let import_string = subcommand_matches.value_of("import-string").unwrap();
    let import_config = ::data_encoding::BASE32_NOPAD.decode(import_string.as_bytes());
    if import_config.is_err() {
        error!("Failed to decode import string");
        ::std::process::exit(1);
    }
    let import_config = import_config.unwrap();
    let import_config = String::from_utf8(import_config);
    if import_config.is_err() {
        error!("Failed to decode import string");
        ::std::process::exit(1);
    }
    let import_config = import_config.unwrap();
    import_config.parse::<toml::Value>().unwrap()
}

fn subcommand_learn_server() {
    let subcommand_matches = crate::arg::matches().subcommand().1.unwrap();
    let name = subcommand_matches.value_of("name").unwrap();
    let mut import_config = load_import_config();

    let mut dh = ::snow::CryptoResolver::resolve_dh(&::snow::DefaultResolver, &::snow::params::DHChoice::Curve25519).unwrap();
    let mut rng = ::snow::CryptoResolver::resolve_rng(&::snow::DefaultResolver).unwrap();
    ::snow::types::Dh::generate(&mut *dh, &mut *rng);

    let privkey = ::snow::types::Dh::privkey(&*dh);
    let pubkey = ::snow::types::Dh::pubkey(&*dh);
    let mut psk = [0u8; 32];
    ::snow::types::Random::fill_bytes(&mut *rng, &mut psk);

    import_config
        .as_table_mut()
        .unwrap()
        .insert("privkey".to_string(), ::data_encoding::BASE32_NOPAD.encode(&privkey).into());
    import_config
        .as_table_mut()
        .unwrap()
        .insert("my_pubkey".to_string(), ::data_encoding::BASE32_NOPAD.encode(&pubkey).into());
    import_config
        .as_table_mut()
        .unwrap()
        .insert("psk".to_string(), ::data_encoding::BASE32_NOPAD.encode(&psk).into());
    import_config.as_table_mut().unwrap().insert("name".to_string(), name.into());

    let config_path = subcommand_matches.value_of("config").unwrap();
    let config_path = resolve_path(config_path);
    let config_exists = ::std::fs::metadata(&config_path).is_ok();
    let (mut old_config, passphrase): (::toml::Value, Option<String>) = if config_exists {
        load_file(&config_path).unwrap()
    } else {
        ("".parse().unwrap(), None)
    };

    drop_server_named(&mut old_config, name);

    if old_config.as_table().unwrap().get("servers").is_none() {
        old_config
            .as_table_mut()
            .unwrap()
            .insert("servers".to_string(), ::toml::Value::Array(vec![]));
    }

    old_config
        .as_table_mut()
        .unwrap()
        .get_mut("servers")
        .unwrap()
        .as_array_mut()
        .unwrap()
        .push(import_config);

    let mut display_config = ::toml::value::Table::new();
    display_config.insert("psk".to_string(), ::data_encoding::BASE32_NOPAD.encode(&psk).into());
    display_config.insert("pubkey".to_string(), ::data_encoding::BASE32_NOPAD.encode(&pubkey).into());

    let export_string = ::data_encoding::BASE32_NOPAD.encode(::toml::to_string(&display_config).unwrap().as_bytes());

    println!("{}", ::toml::Value::Table(display_config));
    println!("Import this client: {}", export_string);

    save_config(&config_path, old_config, passphrase.as_ref().map(|x| x.as_str()));
}

fn subcommand_learn_client() {
    let subcommand_matches = crate::arg::matches().subcommand().1.unwrap();
    let mut import_config = load_import_config();

    for field in &["name", "setuser", "forcedcommand"] {
        import_config.as_table_mut().unwrap().remove(*field);
        if let Some(val) = subcommand_matches.value_of(field) {
            import_config.as_table_mut().unwrap().insert(field.to_string(), val.into());
        }
    }

    let config_path = subcommand_matches.value_of("config").unwrap();
    let (mut old_config, passphrase) = load_file(&config_path).unwrap();
    {
        let clients = old_config.as_table_mut().unwrap().entry("clients".to_string());
        clients
            .or_insert_with(|| ::toml::Value::Array(vec![]))
            .as_array_mut()
            .unwrap()
            .push(import_config);
    }

    save_config(&config_path, old_config, passphrase.as_ref().map(|x| x.as_str()));
    info!("Client added.");
}

fn subcommand_delete_client() {
    subcommand_delete_client_or_server("clients");
}

fn subcommand_delete_server() {
    subcommand_delete_client_or_server("servers");
}

fn subcommand_delete_client_or_server(which: &str) {
    assert!(["clients", "servers"].contains(&which));
    let subcommand_matches = crate::arg::matches().subcommand().1.unwrap();
    let name = subcommand_matches.value_of("name");
    let pubkey = subcommand_matches.value_of("pubkey");
    let config_path = subcommand_matches.value_of("config").unwrap();
    let (mut old_config, passphrase) = load_file(&config_path).unwrap();
    let orig_len = old_config
        .as_table_mut()
        .unwrap()
        .get_mut(which)
        .map(|x| x.as_array().unwrap().len())
        .unwrap_or(0);
    if let Some(entries) = old_config.as_table_mut().unwrap().get_mut(which) {
        if let Some(pubkey) = pubkey {
            entries
                .as_array_mut()
                .unwrap()
                .retain(|x| x.get("pubkey").is_none() || x.get("pubkey").unwrap().as_str().unwrap() != pubkey);
        }
        if let Some(name) = name {
            entries
                .as_array_mut()
                .unwrap()
                .retain(|x| x.get("name").is_none() || x.get("name").unwrap().as_str().unwrap() != name);
        }
    }
    let new_len = old_config
        .as_table_mut()
        .unwrap()
        .get_mut(which)
        .map(|x| x.as_array().unwrap().len())
        .unwrap_or(0);
    let delta = orig_len.checked_sub(new_len).unwrap();
    save_config(&config_path, old_config, passphrase.as_ref().map(|x| x.as_str()));
    info!("Deleted {} {}.", delta, which);
}
