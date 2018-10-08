#[derive(Serialize, Deserialize, Debug)]
enum InnerMessage {
    Dummy {},
    ProtocolVersionAnnounce { version: u64 },
    Rekey { new_material: Vec<u8> },
    Reject { message_number: u64, note: String },
    Accept { message_number: u64 },
}
