use std::env;
use std::fs::{read, write};

use lzari::LZARIContext;

fn main() {
    let mut args = env::args();
    let prog = args.next().unwrap();
    let mode = args.next().unwrap();
    let infile = args.next().unwrap();
    let outfile = args.next().unwrap();

    let infile = read(infile).unwrap();

    let lzari = LZARIContext::new(&infile);

    let out = match mode.as_str() {
        "e" | "E" => lzari.encode(),
        "d" | "D" => lzari.decode(),
        _ => panic!("{prog}: invalid mode {mode}"),
    };

    write(outfile, out).unwrap();
}
