//! Lists files of a BSA archive matching a substring, and reads the first match.
//!
//! ```sh
//! cargo run -p serde_bsa --example bsa_list -- "Skyrim - Animations.bsa" actors/cow
//! ```
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args().skip(1);
    let path = args.next().ok_or("usage: bsa_list <archive.bsa> [filter]")?;
    let filter = args.next().unwrap_or_default().to_lowercase();

    let archive = serde_bsa::Archive::open(path)?;
    let mut matches: Vec<_> = archive.paths().filter(|p| p.contains(&filter)).collect();
    matches.sort_unstable();
    println!("{} files match", matches.len());
    let limit = if std::env::var_os("BSA_LIST_ALL").is_some() { usize::MAX } else { 20 };
    for path in matches.iter().take(limit) {
        println!("  {path}");
    }
    if let Some(first) = matches.first() {
        let bytes = archive.read(first)?.unwrap_or_default();
        println!("read {first}: {} bytes, head {:02x?}", bytes.len(), &bytes[..bytes.len().min(16)]);
        if let Some(out) = std::env::var_os("BSA_EXTRACT") {
            std::fs::write(&out, &bytes)?;
            println!("extracted to {}", out.to_string_lossy());
        }
    }
    Ok(())
}
