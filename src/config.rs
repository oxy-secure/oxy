//! This module contains the schema for config files. A given oxy instance is
//! powered entirely off of a config file, and all command line arguments are
//! translated into config file entries before being used at runtime.

/// An Oxy config.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Config {
    /// The mode of the instance that will be spawned off this config file.
    /// Usually filled in by cli arguments rather than an actual config file
    /// entry.
    pub mode: Option<Mode>,
    /// The outer encryption key. See security note in
    /// Oxy::outer_decrypt_packet.
    #[serde(with = "crate::base32")]
    #[serde(default)]
    pub outer_key: Option<Vec<u8>>,
    /// The server to connect to as a client.
    pub destination: Option<String>,
}

/// The modes in which an Oxy instance can run.
#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
pub enum Mode {
    /// A server that manages a UDP socket and passes the data forward to the
    /// relevant connection processor Oxy instance. Creates connection
    /// processor Oxy instances as needed.
    Server,
    /// The server side of a single connection.
    ServerConnection,
    /// The client side of a single connection.
    Client,
}
