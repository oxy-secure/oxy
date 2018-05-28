#[derive(Serialize, Deserialize, Debug, Clone)]
#[cfg_attr(rustfmt, rustfmt::skip)]
pub enum OxyMessage {
	DummyMessage { data: Vec<u8> },
	BasicCommand { command: String },
	BasicCommandOutput { stdout: Vec<u8>, stderr: Vec<u8> },
	Reject { message_number: u64 },
	PtyRequest { command: String },
	PtyRequestResponse { granted: bool },
	PtySizeAdvertisement { w: u16, h: u16 },
	PtyInput { data: Vec<u8> },
	PtyOutput { data: Vec<u8> },
	DownloadRequest { path: String },
	UploadRequest { path: String },
	FileSize { reference: u64, size: u64 },
	FileData { reference: u64, data: Vec<u8> },
	RemoteOpen { addr: String },
	RemoteBind { addr: String },
    RemoteStreamData { reference: u64, data: Vec<u8> },
    LocalStreamData { reference: u64, data: Vec<u8> },
	BindConnectionAccepted { reference: u64 },
    TunnelRequest { tap: bool, name: String },
    TunnelData { reference: u64, data: Vec<u8> },
}
