use super::*;
use hayro::vello_cpu::Pixmap;
use sqlx::{Row, SqlitePool};
use std::{io::Cursor, path::PathBuf};

// A colored first page and a different second page prove this is a real,
// first-page-only raster rather than a title card or an extracted random image.
fn document(page_properties: &str) -> Vec<u8> {
    let streams = [
        "0.2 0.4 0.8 rg 0 0 200 300 re f 0 0 0 rg BT /F1 24 Tf 20 240 Td (PDF Cover) Tj ET",
        "0 1 0 rg 0 0 200 300 re f BT /F1 12 Tf 20 240 Td (Second page) Tj ET",
    ];
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".into(),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 300] {page_properties} /Resources << /Font << /F1 7 0 R >> >> /Contents 5 0 R >>"),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 300] /Resources << /Font << /F1 7 0 R >> >> /Contents 6 0 R >>".into(),
    ];
    for stream in streams {
        objects.push(format!(
            "<< /Length {} >>\nstream\n{stream}\nendstream",
            stream.len()
        ));
    }
    objects.push("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into());
    let mut bytes = b"%PDF-1.4\n".to_vec();
    let mut offsets = vec![0];
    for (i, object) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", i + 1).as_bytes());
    }
    let start = bytes.len();
    bytes.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len()).as_bytes());
    for offset in &offsets[1..] {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!(
            "trailer\n<< /Root 1 0 R /Size {} >>\nstartxref\n{start}\n%%EOF\n",
            offsets.len()
        )
        .as_bytes(),
    );
    bytes
}

struct Case {
    root: PathBuf,
    source: PathBuf,
    data: PathBuf,
    db: crate::Db,
    rt: tokio::runtime::Runtime,
    pool: SqlitePool,
}
impl Case {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("readwise-pdf-cover-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let source = root.join("中文 cover.PDF");
        fs::write(&source, document("")).unwrap();
        let data = root.join("data");
        let db = crate::Db::open_in(data.clone()).unwrap();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let pool = rt
            .block_on(SqlitePool::connect(&format!(
                "sqlite:{}",
                data.join("database/app.db").display()
            )))
            .unwrap();
        Self {
            root,
            source,
            data,
            db,
            rt,
            pool,
        }
    }
    fn sql(&self, sql: &str) {
        self.rt
            .block_on(sqlx::raw_sql(sql).execute(&self.pool))
            .unwrap();
    }
    fn reader_state(&self) -> (String, i64, i64, String, String, i64) {
        self.rt.block_on(sqlx::query_as("SELECT s.status, s.progress_current, s.progress_total, s.location, s.metadata, b.updated_at FROM book_status s JOIN books b ON b.id=s.book_id LIMIT 1").fetch_one(&self.pool)).unwrap()
    }
}
impl Drop for Case {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn import_creates_real_first_page_png_and_keeps_pdf_and_text() {
    let c = Case::new();
    let original = fs::read(&c.source).unwrap();
    let book = c.db.import_book(&c.source).unwrap();
    let cover = c.data.join(book.cover_path.unwrap());
    let bytes = fs::read(cover).unwrap();
    let pixmap = Pixmap::from_png(Cursor::new(bytes)).unwrap();
    assert_eq!((pixmap.width(), pixmap.height()), (400, 600));
    let pixels = pixmap.data_as_u8_slice();
    assert!(pixels.chunks_exact(4).all(|p| p[3] == 255));
    assert!(
        pixels
            .chunks_exact(4)
            .filter(|p| p[0] < 30 && p[1] < 30 && p[2] < 30)
            .count()
            > 100
    );
    assert!(
        pixels
            .chunks_exact(4)
            .filter(|p| (p[0] as i16 - 51).abs() < 3
                && (p[1] as i16 - 102).abs() < 3
                && (p[2] as i16 - 204).abs() < 3)
            .count()
            > 200_000
    );
    assert_eq!(fs::read(&c.source).unwrap(), original);
    assert_eq!(fs::read(c.data.join(&book.file_path)).unwrap(), original);
    assert!(!c.source.with_extension("png").exists());
    let pages =
        c.db.load_book_chapters(&book.id, &c.data.join(book.file_path))
            .unwrap();
    assert!(pages.chapters[0].html.contains("PDF Cover"));
    assert!(!pages.chapters[0].html.contains(FILE));
}

#[test]
fn thumbnail_respects_crop_rotation_white_background_and_pixel_bound() {
    let c = Case::new();
    for (props, expected) in [
        ("/Rotate 90", (600, 400)),
        ("/CropBox [20 20 120 220] /Rotate 90", (600, 300)),
    ] {
        fs::write(&c.source, document(props)).unwrap();
        let image = Pixmap::from_png(Cursor::new(generate(&c.source).unwrap())).unwrap();
        assert_eq!((image.width(), image.height()), expected);
    }
    fs::write(
        &c.source,
        super::super::tests::pdf_with_font(
            &["BT /F1 14 Tf 20 200 Td (Small text) Tj ET"],
            false,
            "Helvetica",
        ),
    )
    .unwrap();
    let image = Pixmap::from_png(Cursor::new(generate(&c.source).unwrap())).unwrap();
    assert!(
        image
            .data_as_u8_slice()
            .chunks_exact(4)
            .filter(|p| *p == [255, 255, 255, 255])
            .count()
            > 200_000
    );
}

#[test]
fn backfill_is_idempotent_preserves_existing_covers_and_reader_coordinates() {
    let c = Case::new();
    let book = c.db.import_book(&c.source).unwrap();
    let file = c.data.join(&book.file_path);
    let snapshot = file.parent().unwrap().join(super::super::SNAPSHOT_FILE);
    let before = fs::read(&snapshot).unwrap();
    c.sql("UPDATE books SET cover_path=NULL, tags='[\"Rust\"]'; UPDATE book_status SET status='reading', progress_current=78, progress_total=100, location='v3:1:0:0', metadata='{\"want_to_read\":true}';");
    c.db.add_bookmark(&book.id, "v3:1:0:0", "Keep position")
        .unwrap();
    let state = c.reader_state();
    let original = fs::read(&c.source).unwrap();
    let mut ready = Vec::new();
    assert_eq!(
        c.db.backfill_pdf_covers(|id, path| ready.push((id, path)))
            .unwrap(),
        1
    );
    assert_eq!(ready[0].0, book.id);
    assert!(Path::new(&ready[0].1).is_file());
    assert_eq!(c.reader_state(), state);
    assert_eq!(fs::read(&snapshot).unwrap(), before);
    assert_eq!(fs::read(&c.source).unwrap(), original);
    assert_eq!(c.db.get_shelf_items().unwrap()[0].tags, ["Rust"]);
    assert_eq!(c.db.get_bookmarks(&book.id).unwrap().len(), 1);
    let cover_before = fs::read(&ready[0].1).unwrap();
    assert_eq!(
        c.db.backfill_pdf_covers(|_, _| panic!("already covered"))
            .unwrap(),
        0
    );
    assert_eq!(fs::read(&ready[0].1).unwrap(), cover_before);
    let custom = file.parent().unwrap().join("custom.png");
    fs::write(&custom, &cover_before).unwrap();
    c.sql(&format!(
        "UPDATE books SET cover_path='books/{}/custom.png'",
        book.id
    ));
    assert_eq!(
        c.db.backfill_pdf_covers(|_, _| panic!("custom cover"))
            .unwrap(),
        0
    );
    assert_eq!(fs::read(custom).unwrap(), cover_before);
    assert!(!fs::read_dir(file.parent().unwrap()).unwrap().any(|p| p
        .unwrap()
        .file_name()
        .to_string_lossy()
        .ends_with(".tmp")));
}

#[test]
fn missing_cover_is_repaired_and_bad_pdf_does_not_block_other_books() {
    let c = Case::new();
    let first = c.db.import_book(&c.source).unwrap();
    let second = c.db.import_book(&c.source).unwrap();
    fs::remove_file(c.data.join(first.cover_path.as_ref().unwrap())).unwrap();
    fs::remove_file(c.data.join(second.cover_path.as_ref().unwrap())).unwrap();
    fs::write(c.data.join(first.file_path), b"not a PDF").unwrap();
    let mut ready = Vec::new();
    assert_eq!(c.db.backfill_pdf_covers(|id, _| ready.push(id)).unwrap(), 1);
    assert_eq!(ready, [second.id]);
    assert!(c.data.join(second.cover_path.unwrap()).is_file());
}

#[test]
fn cover_write_failure_cleans_temporary_file_and_never_removes_source() {
    let c = Case::new();
    let output = c.root.join("output");
    fs::create_dir_all(output.join(FILE)).unwrap(); // Force rename failure.
    assert!(save(&c.source, &output).is_err());
    assert_eq!(fs::read_dir(output).unwrap().count(), 1);
    assert!(c.source.is_file());
    assert!(save(&c.source, &c.root.join("removed-book")).is_err());
    assert!(!c.root.join("removed-book").exists());
}

#[test]
fn failed_cover_database_update_preserves_reading_state_and_can_retry() {
    let c = Case::new();
    c.db.import_book(&c.source).unwrap();
    c.sql("UPDATE books SET cover_path=NULL; UPDATE book_status SET metadata='{}'; CREATE TRIGGER reject_cover BEFORE UPDATE OF cover_path ON books BEGIN SELECT RAISE(ABORT, 'test failure'); END;");
    let before = c.reader_state();
    assert_eq!(
        c.db.backfill_pdf_covers(|_, _| panic!("not committed"))
            .unwrap(),
        0
    );
    assert_eq!(c.reader_state(), before);
    let row =
        c.rt.block_on(sqlx::query("SELECT cover_path FROM books").fetch_one(&c.pool))
            .unwrap();
    assert!(row.get::<Option<String>, _>("cover_path").is_none());
    c.sql("DROP TRIGGER reject_cover");
    assert_eq!(c.db.backfill_pdf_covers(|_, _| {}).unwrap(), 1);
}
