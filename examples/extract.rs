//! Extracts files from an archive.
//!
//! To extract all files from an archive into the current directory, run:
//!
//! ```shell
//! cargo run --example extract <path/to/archive.a>
//! ```
//!
//! This is roughly equivalent to running:
//!
//! ```shell
//! ar -x <path/to/archive.a>
//! ```

use std::env;
use std::fs::File;
use std::io;
use std::path::Path;
use std::str;

fn main() {
    let num_args = env::args().count();
    if num_args != 2 {
        println!("Usage: extract <path/to/archive.a>");
        return;
    }

    let input_path = env::args().nth(1).unwrap();
    let input_path = Path::new(&input_path);
    let input_file =
        File::open(input_path).expect("failed to open input file");
    let mut archive = tokio_ar::Archive::new(input_file);

    while let Some(entry) = archive.next_entry() {
        let mut entry = entry.expect("failed to parse archive entry");
        let output_path = Path::new(
            str::from_utf8(entry.header().identifier())
                .expect("Non UTF-8 filename"),
        )
        .to_path_buf();
        let mut output_file =
            File::create(&output_path).unwrap_or_else(|_| {
                panic!("unable to create file {:?}", output_path)
            });
        io::copy(&mut entry, &mut output_file).unwrap_or_else(|_| {
            panic!("failed to extract file {:?}", output_path)
        });
    }
}
