//! Dictionary generator for the input method (runs on the device at install time; see
//! `packaging/ime_install.sh`). Converts Mozc OSS dictionary text into `harbour-ime.dic`.
//!
//!   harbour_ime_dict build <out.dic> <dictionary00.txt> [more.txt ...]
//!   harbour_ime_dict verify <file.dic>
//!   harbour_ime_dict lookup <file.dic> <hiragana>
//!   harbour_ime_dict convert <file.dic> <hiragana>     (segmenter, for checking)
//!
//! The text files are the user's downloaded data; nothing here is shipped with the launcher.

use harbour_shell::ime::conv::Converter;
use harbour_shell::ime::dict::{build_from_mozc_files, BuildOpts, Dict};
use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let code = match run(&args[1..]) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("harbour_ime_dict: {e}");
            1
        }
    };
    std::process::exit(code);
}

fn run(args: &[String]) -> Result<(), String> {
    match args.first().map(|s| s.as_str()) {
        Some("build") if args.len() >= 3 => {
            let inputs: Vec<std::path::PathBuf> = args[2..].iter().map(std::path::PathBuf::from).collect();
            let (keys, bytes, _) = build_from_mozc_files(&inputs, &BuildOpts::default(), Path::new(&args[1]))?;
            println!("{}: {} readings, {} KiB", args[1], keys, bytes / 1024);
            Ok(())
        }
        Some("verify") if args.len() == 2 => {
            let d = Dict::open(Path::new(&args[1]))?;
            if d.verify() {
                println!("ok: {} readings", d.len());
                Ok(())
            } else {
                Err("checksum mismatch".into())
            }
        }
        Some("lookup") if args.len() == 3 => {
            let d = Dict::open(Path::new(&args[1]))?;
            for c in d.lookup(&args[2]) {
                println!("{}\t{}", c.cost, c.surface);
            }
            Ok(())
        }
        Some("convert") if args.len() == 3 => {
            let conv = Converter { dict: Some(Dict::open(Path::new(&args[1]))?), ..Default::default() };
            for c in conv.candidates(&args[2]) {
                println!("{c}");
            }
            Ok(())
        }
        _ => Err("usage: harbour_ime_dict build <out.dic> <mozc-dictionary.txt>... | verify <dic> | lookup <dic> <hira> | convert <dic> <hira>".into()),
    }
}
