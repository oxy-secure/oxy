use textwrap::{fill, termwidth};

pub fn print_guide() -> ! {
    println!("{}", fill(include_str!("guide.txt"), termwidth()));
    ::std::process::exit(0);
}
