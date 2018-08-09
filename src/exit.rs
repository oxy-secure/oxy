use std::cell::RefCell;

thread_local! {
    static EXIT_HOOKS: RefCell<Vec<Box<dyn Fn() -> ()>>> = RefCell::new(Vec::new());
}

pub(crate) fn exit(status: i32) -> ! {
    let mut hooks = Vec::new();
    EXIT_HOOKS.with(|x| ::std::mem::swap(&mut hooks, &mut *x.borrow_mut()));
    for hook in hooks {
        (hook)();
    }
    ::std::process::exit(status);
}

pub(crate) fn push_hook<T: Fn() -> () + 'static>(callback: T) {
    let callback = Box::new(callback);
    EXIT_HOOKS.with(|x| x.borrow_mut().push(callback));
}
