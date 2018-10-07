fn main() {
    let args: Vec<String> = ::std::env::args().collect();
    let args2: Vec<&str> = args.iter().map(|x| x.as_str()).collect();
    ::oxy::entry::run_args(&args2[..]);
}
