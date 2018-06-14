use clap::ArgMatches;
use transportation::EncryptionPerspective::{self, Alice, Bob};

pub(crate) struct OxyArg {
    matches: ArgMatches<'static>,
}

impl OxyArg {
    pub fn create(args: Vec<String>) -> OxyArg {
        let app = crate::arg::create_app();
        let matches = app.get_matches_from(&args);
        OxyArg { matches }
    }

    pub fn mode(&self) -> String {
        self.matches.subcommand_name().unwrap().to_string()
    }

    pub fn perspective(&self) -> EncryptionPerspective {
        match self.mode().as_str() {
            "reexec" => Bob,
            "server" => Bob,
            "serve-one" => Bob,
            "reverse-server" => Bob,
            _ => Alice,
        }
    }
}
