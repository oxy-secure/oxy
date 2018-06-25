use crate::{
    core::Oxy,
    message::OxyMessage::{self, *},
};
#[allow(unused_imports)]
use log::{debug, error, info, log, trace, warn};

impl Oxy {
    crate fn restrict_message(&self, message: OxyMessage) -> Result<OxyMessage, ()> {
        if self.perspective() == ::transportation::EncryptionPerspective::Alice {
            return Ok(message);
        }
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
        let forced_command = crate::conf::forced_command(self.peer().as_ref().map(|x| x.as_str()));
        if forced_command.is_none() {
            return Ok(message);
        }
        let forced_command = forced_command.unwrap();
        let forced_command = ::shlex::split(&forced_command);
        if forced_command.is_none() {
            error!("Failed to parse forced command value.");
            ::std::process::exit(1);
        }
        let forced_command = forced_command.unwrap();
        debug!("Processing restriction: forced_command: {:?}", forced_command);

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
