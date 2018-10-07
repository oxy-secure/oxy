//! This module contains entry points for the application.

/// Execute a loaded config.
pub fn run_config(config: &crate::config::Config) {
    crate::oxy::Oxy::new(config);
    ::transportation::run();
}

/// Generate a config file based on command line arguments, then execute that
/// config file. If the command line arguments specify a config file it will be
/// loaded and integrated with the other arguments.
pub fn run_args<T>(args: &[&T])
where
    T: AsRef<str> + ?Sized,
{
    run_config(
        &crate::arg::args_to_config(args)
            .map_err(|x| x.exit())
            .expect("impossible"),
    )
}
