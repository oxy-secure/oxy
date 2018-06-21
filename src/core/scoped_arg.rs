use clap::ArgMatches;
#[allow(unused_imports)]
use log::{debug, error, info, log, trace, warn};
use transportation::EncryptionPerspective::{self, Alice, Bob};

pub(crate) struct OxyArg {
    matches: ArgMatches<'static>,
}

impl OxyArg {
    crate fn create(args: Vec<String>) -> OxyArg {
        let matches = OxyArg::make_matches(args);
        OxyArg { matches }
    }

    crate fn make_matches(mut args: Vec<String>) -> ArgMatches<'static> {
        if let Ok(matches) = crate::arg::create_app().get_matches_from_safe(&args) {
            return matches;
        }
        trace!("Trying implicit 'client'");
        args.insert(1, "client".to_string());
        if let Ok(matches) = crate::arg::create_app().get_matches_from_safe(&args) {
            return matches;
        }
        crate::arg::create_app().get_matches_from(&args)
    }

    crate fn mode(&self) -> String {
        self.matches.subcommand_name().unwrap().to_string()
    }

    crate fn matches<R>(&self, callback: impl FnOnce(&ArgMatches<'static>) -> R) -> R {
        (callback)(self.matches.subcommand_matches(self.mode()).unwrap())
    }

    crate fn perspective(&self) -> EncryptionPerspective {
        match self.mode().as_str() {
            "reexec" => Bob,
            "server" => Bob,
            "serve-one" => Bob,
            "reverse-server" => Bob,
            _ => Alice,
        }
    }
}
