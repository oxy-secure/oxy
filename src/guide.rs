use textwrap::{fill, termwidth};

crate fn print_guide() -> ! {
    println!("{}", fill(include_str!("guide.txt"), termwidth()));
    ::std::process::exit(0);
}
