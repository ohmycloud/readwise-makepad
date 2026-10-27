//! Diagnostic: ONLY use a copied QA data directory (load may migrate snapshots).
use std::{path::PathBuf, time::Instant};
fn main() {
    let mut args = std::env::args().skip(1);
    let data = PathBuf::from(args.next().unwrap());
    let needle = args.next().unwrap();
    let db = readwise_core::Db::open_in(data.clone()).unwrap();
    let mut page_override = None;
    let mut uncached = false;
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--uncached" => uncached = true,
            "--import" => {
                let source = PathBuf::from(args.next().expect("--import PDF"));
                let start = Instant::now();
                db.import_book(&source).unwrap();
                println!("import={:.2}ms", start.elapsed().as_secs_f64() * 1000.);
            }
            "--page" => {
                page_override = Some(args.next().expect("--page INDEX").parse::<usize>().unwrap())
            }
            _ => panic!("unknown flag: {flag}"),
        }
    }
    let item = db
        .get_shelf_items()
        .unwrap()
        .into_iter()
        .find(|b| {
            b.title.contains(&needle)
                && b.file_abs_path.ends_with(".pdf")
                && std::path::Path::new(&b.file_abs_path).exists()
        })
        .unwrap();
    for run in 0..2 {
        let all = Instant::now();
        let start = Instant::now();
        let loaded = db
            .load_book_chapters(&item.id, std::path::Path::new(&item.file_abs_path))
            .unwrap();
        println!(
            "run={run} core={:.2}ms pages={}",
            start.elapsed().as_secs_f64() * 1000.,
            loaded.chapters.len()
        );
        let start = Instant::now();
        let s = db.get_status(&item.id).unwrap();
        let saved_page = s
            .and_then(|s| {
                s.location
                    .split(':')
                    .nth(1)
                    .and_then(|p| p.parse::<usize>().ok())
            })
            .unwrap_or(0);
        let page = page_override.unwrap_or(saved_page);
        let _ = db.gloss_context(Some(&item.id)).unwrap();
        println!(
            "run={run} status+gloss={:.2}ms page={page}",
            start.elapsed().as_secs_f64() * 1000.
        );
        let path = std::path::Path::new(&item.file_abs_path);
        let session = if uncached {
            readwise_math::PdfSession::new(path)
        } else {
            readwise_math::PdfSession::with_cache(
                path,
                path.parent().unwrap().join("pdf-visual-cache"),
            )
        };
        let blocks = readwise_core::books::epub_meta::split_blocks(&loaded.chapters[page].html);
        let texts = blocks
            .iter()
            .map(|b| readwise_core::vocab::annotate::rendered_text(b))
            .collect::<Vec<_>>();
        let start = Instant::now();
        let p = session.page(page, &blocks, &texts);
        println!(
            "run={run} math={:.2}ms bytes={} total={:.2}ms",
            start.elapsed().as_secs_f64() * 1000.,
            p.bytes,
            all.elapsed().as_secs_f64() * 1000.
        );
    }
}
