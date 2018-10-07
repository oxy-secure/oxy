//! These functions are designed to be cfg'd into nops. I'm still not really
//! decided on a logging strategy, this is just a placeholder that will
//! hopefully be efficient to swap out later.

impl crate::oxy::Oxy {
    pub(crate) fn warn<T, R>(&self, callback: T)
    where
        T: FnOnce() -> R,
        R: ::std::fmt::Display,
    {
        println!("WARN: {}", (callback)());
    }

    pub(crate) fn info<T, R>(&self, callback: T)
    where
        T: FnOnce() -> R,
        R: ::std::fmt::Display,
    {
        println!("INFO: {}", (callback)());
    }
}
