extern crate data_encoding;
extern crate parking_lot;
extern crate transportation;

use parking_lot::Mutex;
use std::{
    fs::{metadata, remove_file, File},
    io::Read,
    process::{Child, Command, Stdio},
    thread::sleep,
    time::Duration,
};
use transportation::ring::rand::SecureRandom;

static SERIAL_TESTS: Mutex<()> = Mutex::new(());

fn mk_identity() -> String {
    let mut identity = [0u8; 36];
    transportation::RNG.fill(&mut identity).unwrap();
    let identity = data_encoding::BASE32_NOPAD.encode(&identity);
    format!("--identity={}", identity)
}

fn binpath() -> String {
    "./target/debug/oxy".to_string()
}

fn hold() {
    sleep(Duration::from_secs(1));
}

fn system(cmd: &str) -> Child {
    Command::new("bash").arg("-c").arg(cmd).spawn().unwrap()
}

#[test]
#[cfg(unix)]
fn portfwd_l() {
    portfwd("L");
}

#[test]
#[cfg(unix)]
fn portfwd_r() {
    portfwd("R");
}

#[cfg(unix)]
fn portfwd(spec: &str) {
    let _guard = SERIAL_TESTS.lock();
    let identity = mk_identity();
    let mut server = Command::new(&binpath()).args(&["serve-one", &identity]).spawn().unwrap();
    hold();
    let spec = format!("{} 127.0.0.1:34614 127.0.0.1:44614", spec);
    let mut client = Command::new(&binpath())
        .args(&["client", "127.0.0.1:2600", &identity, "-m", &spec])
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    hold();
    let mut ncat_listener = system("ncat --ssl -l 44614 >/tmp/oxy-test-portfwd");
    hold();
    let mut ncat_sender = system("ncat --ssl 127.0.0.1 34614 < <(echo -n abcdef)");
    hold();
    server.kill().ok();
    client.kill().ok();
    ncat_listener.kill().ok();
    ncat_sender.kill().ok();
    assert_eq!(metadata("/tmp/oxy-test-portfwd").unwrap().len(), 6);
    let mut read_buf = Vec::new();
    File::open("/tmp/oxy-test-portfwd").unwrap().read_to_end(&mut read_buf).unwrap();
    assert_eq!(&read_buf, b"abcdef");
    remove_file("/tmp/oxy-test-portfwd").unwrap();
}

#[test]
fn catpty() {
    let _guard = SERIAL_TESTS.lock();
    let identity = mk_identity();
    let mut server = Command::new(&binpath()).args(&["server", &identity]).spawn().unwrap();
    hold();
    let meta = r#"pty "stty raw; echo -n -e '\x00\x10\x0a\xff\x0d\x41'""#;
    let output = Command::new(&binpath())
        .args(&["client", "127.0.0.1:2600", &identity, "-m", meta])
        .output()
        .unwrap();
    server.kill().unwrap();
    assert_eq!(&output.stdout[..], b"\x00\x10\n\xff\rA");
}

#[test]
fn catpty_rev() {
    let _guard = SERIAL_TESTS.lock();
    let identity = mk_identity();
    let meta = r#"pty "stty raw; echo -n -e '\x00\x10\x0a\xff\x0d\x42'""#;
    let client = Command::new(&binpath())
        .args(&["reverse-client", "127.0.0.1:2600", &identity, "-m", meta])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    hold();
    let _server = Command::new(&binpath())
        .args(&["reverse-server", "127.0.0.1:2600", &identity])
        .spawn()
        .unwrap();
    let output = client.wait_with_output().unwrap();
    assert_eq!(&output.stdout[..], b"\x00\x10\n\xff\rB");
}

#[test]
#[cfg(unix)]
fn copy_single_file() {
    let _guard = SERIAL_TESTS.lock();
    let identity = mk_identity();
    let mut server = Command::new("./target/debug/oxy").arg("serve-one").arg(&identity).spawn().unwrap();
    hold();
    let mut client = Command::new("./target/debug/oxy")
        .arg("copy")
        .arg("localhost:/etc/hosts")
        .arg("/tmp/")
        .arg(&identity)
        .spawn()
        .unwrap();
    hold();
    hold();
    hold();
    server.kill().ok();
    client.kill().ok();
    assert_eq!(metadata("/etc/hosts").unwrap().len(), metadata("/tmp/hosts").unwrap().len());
    remove_file("/tmp/hosts").unwrap();
}
