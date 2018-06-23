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
        let forced_command = crate::arg::matches().value_of("forced command");
        if forced_command.is_none() {
            return Ok(message);
        }
        let forced_command = forced_command.unwrap().to_string();
        debug!("Processing restrictions");

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
