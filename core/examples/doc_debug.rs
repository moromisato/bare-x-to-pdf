fn main() {
    let path = std::env::args().nth(1).expect("usage: doc_debug <file.doc>");
    let bytes = std::fs::read(&path).expect("cannot read input");
    match simple_converter_core::doc::debug(&bytes) {
        Ok(info) => println!("{info}"),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}
