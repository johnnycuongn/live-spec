//! See exactly what Live Spec would retrieve for a draft. Handy for tuning.
//!
//!   cargo run -p livespec-core --example probe -- <project-dir> "<draft text>"

use livespec_core::project::{retrieve, scan};

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(dir), Some(draft)) = (args.next(), args.next()) else {
        eprintln!("usage: probe <project-dir> \"<draft>\"");
        std::process::exit(2);
    };
    let started = std::time::Instant::now();
    let index = scan(std::path::Path::new(&dir)).expect("scan failed");
    println!("{}: {} indexed / {} files in {:?}", index.name, index.files.len(), index.tree.len(), started.elapsed());

    let started = std::time::Instant::now();
    let hits = retrieve(&index, &draft, 4, 6_000);
    println!("retrieval took {:?}\n", started.elapsed());
    for s in hits {
        let first = s.text.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
        println!("{}:{}  {}", s.path, s.start_line, first.trim());
    }
}
