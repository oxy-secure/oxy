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
    /// The server public key
    #[serde(with = "crate::base32")]
    #[serde(default)]
    pub remote_public_key: Option<Vec<u8>>,
    /// The local private key
    #[serde(with = "crate::base32")]
    #[serde(default)]
    pub local_private_key: Option<Vec<u8>>,
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

impl Config {
    /// Used for combining configs to allow for a system-wide config + a user
    /// level config + a CLI argument config, etc.
    pub fn overwrite_with(&mut self, other: &Config) {
        // This is a pretty crude approach that wastes a lot of time and memory, but
        // it's not a hot operation so that's not too bad. What's more of a bummer is
        // that it doesn't allow merging configs with non-serializable fields (Box<Fn()
        // -> ()>) if we add any of those later.
        //
        // Doing something like serde-transcode does could be a lot more efficient and
        // flexible, but seems like a lot of work. Also just literally using
        // serde-transcode to move to/from the recursive enum form would be a
        // reasonable thing to do.
        let src: ::toml::Value =
            ::toml::de::from_str(&::toml::ser::to_string(self).unwrap()).unwrap();
        let mut dest: ::toml::Value =
            ::toml::de::from_str(&::toml::ser::to_string(&other).unwrap()).unwrap();
        merge(&mut dest, &src);
        let result: Config = ::toml::de::from_str(&::toml::ser::to_string(&dest).unwrap()).unwrap();
        ::std::mem::replace(self, result);
    }
}

fn merge(dest: &mut ::toml::Value, src: &::toml::Value) {
    // TODO: I haven't actually run any examples through this yet.
    match (dest, src) {
        (&mut ::toml::Value::Table(ref mut dest), &::toml::Value::Table(ref src)) => {
            for (k, v) in src {
                merge(
                    dest.entry(k.clone())
                        .or_insert(::toml::Value::Boolean(false)),
                    v,
                );
            }
        }
        (dest, src) => {
            *dest = src.clone();
        }
    }
}
