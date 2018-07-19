fn bin_path() -> String {
    "target/debug/oxy".to_string()
}

#[test]
fn config_then_touch() {
    let mut rng = ::snow::CryptoResolver::resolve_rng(&::snow::DefaultResolver).unwrap();
    let mut config_name_bytes = [0u8; 8];
    ::snow::types::Random::fill_bytes(&mut *rng, &mut config_name_bytes);
    let config_name = ::data_encoding::BASE32_NOPAD.encode(&config_name_bytes);
    let server_config_name = format!("test-server-{}", config_name);
    let client_config_name = format!("test-client-{}", config_name);
    let touch_file_name = format!("test-touch-{}", config_name);
    let server_string = ::std::process::Command::new(&bin_path())
        .arg("configure")
        .arg("initialize-server")
        .arg("--config")
        .arg(&server_config_name)
        .output()
        .unwrap()
        .stdout;
    let server_string = ::std::string::String::from_utf8(server_string).unwrap();
    let server_string = server_string
        .split('\n')
        .filter(|x| x.contains("Import "))
        .map(|x| x.split(": ").nth(1).unwrap().to_string())
        .next()
        .unwrap();
    let client_string = ::std::process::Command::new(&bin_path())
        .arg("configure")
        .arg("learn-server")
        .arg("--config")
        .arg(&client_config_name)
        .arg("--name=localhost")
        .arg("--import-string")
        .arg(&server_string)
        .output()
        .unwrap()
        .stdout;
    let client_string = ::std::string::String::from_utf8(client_string).unwrap();
    let client_string = client_string
        .split('\n')
        .filter(|x| x.contains("Import "))
        .map(|x| x.split(": ").nth(1).unwrap().to_string())
        .next()
        .unwrap();
    ::std::process::Command::new(&bin_path())
        .arg("configure")
        .arg("learn-client")
        .arg("--config")
        .arg(&server_config_name)
        .arg("--import-string")
        .arg(&client_string)
        .status()
        .unwrap();
    let mut server_process = ::std::process::Command::new(&bin_path())
        .arg("server")
        .arg("--config")
        .arg(&server_config_name)
        .spawn()
        .unwrap();
    ::std::process::Command::new(&bin_path())
        .arg("localhost")
        .arg("--config")
        .arg(&client_config_name)
        .arg("--")
        .arg("touch")
        .arg(&touch_file_name)
        .status()
        .unwrap();
    let mut failed;
    failed = server_process.kill().is_err();
    failed = ::std::fs::remove_file(&server_config_name).is_err() || failed;
    failed = ::std::fs::remove_file(&client_config_name).is_err() || failed;
    failed = ::std::fs::remove_file(&touch_file_name).is_err() || failed;
    assert!(!failed);
}
