use std::time::SystemTime;

#[derive(Serialize, Deserialize, Debug, Clone)]
#[cfg_attr(rustfmt, rustfmt::skip)]
pub enum OxyMessage {
	DummyMessage { data: Vec<u8> },
	BasicCommand { command: String },
	BasicCommandOutput { stdout: Vec<u8>, stderr: Vec<u8> },
	Reject { message_number: u64, note: String },
	PtyRequest { command: String },
	PtyRequestResponse { granted: bool },
	PtySizeAdvertisement { w: u16, h: u16 },
	PtyInput { data: Vec<u8> },
	PtyOutput { data: Vec<u8> },
	DownloadRequest { path: String },
	UploadRequest { path: String, filepart: String },
	FileSize { reference: u64, size: u64 },
	FileData { reference: u64, data: Vec<u8> },
	RemoteOpen { addr: String },
	RemoteBind { addr: String },
	RemoteStreamData { reference: u64, data: Vec<u8> },
	LocalStreamData { reference: u64, data: Vec<u8> },
	BindConnectionAccepted { reference: u64 },
	TunnelRequest { tap: bool, name: String },
	TunnelData { reference: u64, data: Vec<u8> },
	Ping { },
	Pong { },
	ResolvePathRequest { path: String },
	ResolvePathAnswers { reference: u64, complete: bool, answers: Vec<String> },
	StatRequest { path: String },
	StatResult { reference: u64, len: u64, is_dir: bool, is_file: bool, owner: String, group: String, octal_permissions: u16, atime: Option<SystemTime>, mtime: Option<SystemTime>, ctime: Option<SystemTime> },
	ReadDir { path: String },
	ReadDirResult { reference: u64, complete: bool, answers: Vec<String> },
}
