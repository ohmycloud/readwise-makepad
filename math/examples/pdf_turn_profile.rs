//! Reproducible cold-vs-session projection timings, without modifying a book/DB.
//! cargo run -p readwise-math --example pdf_turn_profile -- path.pdf 1 2 3
//! Page arguments are zero-based. Run with --release for production timings.
use readwise_math::{MathPage, PdfSession};
use std::{path::PathBuf, time::Instant};

fn main() {
    let mut args = std::env::args().skip(1);
    let path = PathBuf::from(args.next().expect("PDF path"));
    let pages: Vec<usize> = args
        .map(|s| s.parse().expect("zero-based page number"))
        .collect();
    assert!(!pages.is_empty(), "supply at least one page number");
    // Prepare immutable source outside both measured paths. No migrations,
    // private snapshots or changes beside the user's source file.
    let doc = pdf_oxide::PdfDocument::open(&path).unwrap();
    let sources: Vec<_> = pages
        .iter()
        .map(|&p| {
            let text = doc.extract_text(p).unwrap();
            let html = format!("<p>{}</p>", html_escape::encode_text(&text));
            (vec![html], vec![text])
        })
        .collect();
    drop(doc);
    let session = PdfSession::new(&path);
    for (label, retained) in [("reopen-per-page", false), ("retained-session", true)] {
        let total = Instant::now();
        for (&page, (blocks, texts)) in pages.iter().zip(&sources) {
            let start = Instant::now();
            let projected = if retained {
                session.page(page, blocks, texts)
            } else {
                MathPage::pdf(&path, page, blocks, texts)
            };
            println!(
                "{label} page={page} elapsed_ms={:.2} projection_bytes={}",
                start.elapsed().as_secs_f64() * 1000.,
                projected.bytes
            );
        }
        println!(
            "{label} total_ms={:.2}",
            total.elapsed().as_secs_f64() * 1000.
        );
    }
}
