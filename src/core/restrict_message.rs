use crate::{
    core::Oxy,
    message::OxyMessage::{self, *},
};
#[allow(unused_imports)]
use log::{debug, error, info, log, trace, warn};

impl Oxy {
    crate fn restrict_message(&self, message: OxyMessage) -> Result<OxyMessage, ()> {
        let message = self.restrict_forcedcommand(message)?;
        let message = self.restrict_portforwards(message)?;
        Ok(message)
    }

    fn restrict_portforwards(&self, message: OxyMessage) -> Result<OxyMessage, ()> {
        ();
        // TODO
        Ok(message)
    }

    fn restrict_forcedcommand(&self, message: OxyMessage) -> Result<OxyMessage, ()> {
        let mut forced_command = self.matches(|x| x.value_of("forced command").map(|x| x.to_string()));
        let su_mode = self.matches(|x| x.is_present("su mode"));
        if su_mode {
            // SECURITYWATCH: shlex.quote should be sufficient to avoid command injection
            // in *nix environments. It's not as good as passing the whole thing to
            // execve() as a separate parameter, but it shooooould be file. On windows...
            // IDK. This might have command injection on Windows.
            #[cfg(windows)]
            panic!();
            let user = ::shlex::quote(self.internal.peer_user.borrow().as_ref().map(|x| x.as_str()).unwrap_or("root")).into_owned();
            forced_command = Some(format!("su - {}", user));
        }
        if forced_command.is_none() {
            return Ok(message);
        }
        debug!("Processing restrictions");
        let forced_command = forced_command.unwrap();

        match message.clone() {
            BasicCommand { .. } => Ok(BasicCommand { command: forced_command }),
            PipeCommand { .. } => Ok(PipeCommand { command: forced_command }),
            PtyRequest { .. } => Ok(PtyRequest {
                command: Some(forced_command),
            }),
            UsernameAdvertisement { .. } => Ok(message),
            PtySizeAdvertisement { .. } => Ok(message),
            PtyInput { .. } => Ok(message),
            Success { .. } => Ok(message),
            Reject { .. } => Ok(message),
            Ping {} => Ok(message),
            Pong {} => Ok(message),
            _ => Err(()),
        }
    }
}
