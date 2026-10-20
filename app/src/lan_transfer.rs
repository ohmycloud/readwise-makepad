//! User-scoped LAN receiver. Streams uploads to private temporary files and
//! delegates EPUB parsing/storage to the existing importer, off the UI thread.
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{self, BufRead, BufReader, Read, Write},
    net::{Ipv4Addr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

pub const MAX_UPLOAD_BYTES: u64 = 200 * 1024 * 1024;
const SESSION_LIFETIME: Duration = Duration::from_secs(30 * 60);
const IO_IDLE_TIMEOUT: Duration = Duration::from_secs(20);
const HEADER_LIMIT: usize = 16 * 1024;

#[cfg(target_os = "android")]
static ANDROID_SESSION: std::sync::Mutex<Option<(String, std::sync::Weak<AtomicBool>)>> =
    std::sync::Mutex::new(None);

#[cfg(target_os = "android")]
pub fn stop_android_session(id: &str) {
    if let Ok(active) = ANDROID_SESSION.lock() {
        if let Some((current, stop)) = active.as_ref() {
            if current == id {
                if let Some(stop) = stop.upgrade() {
                    stop.store(true, Ordering::Release);
                }
            }
        }
    }
}

#[cfg(target_os = "android")]
struct AndroidServiceGuard(String);
#[cfg(target_os = "android")]
impl AndroidServiceGuard {
    fn start(id: &str, stop: &Arc<AtomicBool>, english: bool) -> Result<Self, String> {
        *ANDROID_SESSION.lock().map_err(|e| e.to_string())? =
            Some((id.into(), Arc::downgrade(stop)));
        let guard = Self(id.into());
        crate::android_lan_transfer::start_service(id, english)?;
        Ok(guard)
    }
}
#[cfg(target_os = "android")]
impl Drop for AndroidServiceGuard {
    fn drop(&mut self) {
        crate::android_lan_transfer::stop_service(&self.0);
        if let Ok(mut active) = ANDROID_SESSION.lock() {
            if active.as_ref().is_some_and(|(id, _)| id == &self.0) {
                *active = None;
            }
        }
    }
}

#[derive(Debug)]
pub enum TransferNotice {
    Receiving(String),
    Imported(String),
    Failed(String),
    Stopped,
}

pub struct LanTransfer {
    pub id: String,
    pub urls: Vec<String>,
    stop: Arc<AtomicBool>,
}

impl Drop for LanTransfer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}

impl LanTransfer {
    pub fn start(
        data_dir: &Path,
        english: bool,
        import: impl Fn(&Path) -> Result<String, String> + Send + 'static,
        notice: impl Fn(&str, TransferNotice) + Send + 'static,
    ) -> Result<Self, String> {
        Self::start_on(data_dir, english, local_addresses(), import, notice)
    }

    fn start_on(
        data_dir: &Path,
        english: bool,
        addresses: Vec<Ipv4Addr>,
        import: impl Fn(&Path) -> Result<String, String> + Send + 'static,
        notice: impl Fn(&str, TransferNotice) + Send + 'static,
    ) -> Result<Self, String> {
        if addresses.is_empty() {
            return Err(if english {
                "Connect to Wi-Fi, then try again."
            } else {
                "请先连接 Wi-Fi，再开启局域网传书。"
            }
            .into());
        }
        let listener = TcpListener::bind((Ipv4Addr::UNSPECIFIED, 0)).map_err(|e| e.to_string())?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        let id = uuid::Uuid::new_v4().simple().to_string();
        let urls = addresses
            .iter()
            .map(|ip| format!("http://{ip}:{port}/{id}"))
            .collect();
        let root = data_dir.join("transfer-cache").join(&id);
        fs::create_dir_all(&root).map_err(|e| e.to_string())?;
        let stop = Arc::new(AtomicBool::new(false));
        #[cfg(target_os = "android")]
        let platform_guard = match AndroidServiceGuard::start(&id, &stop, english) {
            Ok(guard) => guard,
            Err(error) => {
                let _ = fs::remove_dir_all(&root);
                return Err(error);
            }
        };
        let worker_stop = stop.clone();
        let worker_id = id.clone();
        let worker_root = root.clone();
        std::thread::Builder::new().name("epub-lan-transfer".into()).spawn(move || {
            #[cfg(target_os = "android")]
            let _platform_guard = platform_guard;
            let _cleanup = TemporaryDirectory(worker_root.clone());
            let deadline = Instant::now() + SESSION_LIFETIME;
            let page = upload_page(english);
            while !worker_stop.load(Ordering::Acquire) && Instant::now() < deadline {
                match listener.accept() {
                    Ok((mut stream, peer)) => {
                        // Never expose a receiver on a public address, even if
                        // a device also has cellular or another public route.
                        if !matches!(peer.ip(), std::net::IpAddr::V4(ip) if ip.is_private() || ip.is_link_local() || ip.is_loopback()) {
                            continue;
                        }
                        let result = serve_connection(&mut stream, &worker_id, &worker_root,
                            &page, english, &worker_stop, deadline, &import,
                            &|event| notice(&worker_id, event));
                        if let Err(error) = result {
                            if !worker_stop.load(Ordering::Acquire) {
                                notice(&worker_id, TransferNotice::Failed(error.message.clone()));
                                let _ = respond(&mut stream, error.status, "application/json", &json_message(false, &error.message));
                            }
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(50)),
                    Err(error) => { notice(&worker_id, TransferNotice::Failed(error.to_string())); break; }
                }
            }
            // Release the listening port before reporting the expired session.
            drop(listener);
            notice(&worker_id, TransferNotice::Stopped);
        }).map_err(|e| {
            let _ = fs::remove_dir_all(root);
            e.to_string()
        })?;
        Ok(Self { id, urls, stop })
    }
}

struct TemporaryDirectory(PathBuf);
impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[derive(Debug)]
struct TransferError {
    status: u16,
    message: String,
}
impl TransferError {
    fn new(status: u16, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
}
impl From<io::Error> for TransferError {
    fn from(e: io::Error) -> Self {
        Self::new(400, e.to_string())
    }
}

struct Request {
    method: String,
    target: String,
    headers: BTreeMap<String, String>,
}

fn read_request(reader: &mut impl BufRead) -> Result<Request, TransferError> {
    // A bounded read prevents a single unterminated header allocating without limit.
    let mut headers = Vec::new();
    while headers.len() < HEADER_LIMIT {
        let mut byte = [0];
        reader.read_exact(&mut byte)?;
        headers.push(byte[0]);
        if headers.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    if !headers.ends_with(b"\r\n\r\n") {
        return Err(TransferError::new(431, "Request headers are too large"));
    }
    let raw = std::str::from_utf8(&headers)
        .map_err(|_| TransferError::new(400, "Invalid request headers"))?;
    let mut lines = raw.split("\r\n");
    let parts: Vec<_> = lines
        .next()
        .unwrap_or_default()
        .split_whitespace()
        .collect();
    if parts.len() != 3 || !matches!(parts[2], "HTTP/1.0" | "HTTP/1.1") {
        return Err(TransferError::new(400, "Invalid HTTP request"));
    }
    let mut parsed = BTreeMap::new();
    for line in lines.take_while(|line| !line.is_empty()) {
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| TransferError::new(400, "Invalid header"))?;
        if parsed
            .insert(name.to_ascii_lowercase(), value.trim().to_string())
            .is_some()
        {
            return Err(TransferError::new(400, "Duplicate header"));
        }
    }
    Ok(Request {
        method: parts[0].into(),
        target: parts[1].into(),
        headers: parsed,
    })
}

fn decoded_filename(value: &str) -> Result<String, TransferError> {
    let mut out = Vec::new();
    let mut bytes = value.as_bytes().iter().copied();
    while let Some(b) = bytes.next() {
        if b == b'%' {
            let hi = bytes.next().and_then(|b| (b as char).to_digit(16));
            let lo = bytes.next().and_then(|b| (b as char).to_digit(16));
            match (hi, lo) {
                (Some(hi), Some(lo)) => out.push((hi * 16 + lo) as u8),
                _ => return Err(TransferError::new(400, "Invalid filename encoding")),
            }
        } else {
            out.push(b);
        }
    }
    let name = String::from_utf8(out).map_err(|_| TransferError::new(400, "Invalid filename"))?;
    if name.len() > 240
        || name
            .chars()
            .any(|c| c.is_control() || matches!(c, '/' | '\\' | ':'))
        || readwise_core::books::supported_format(Path::new(&name)).is_none()
        || name.eq_ignore_ascii_case(".epub")
        || name.eq_ignore_ascii_case(".pdf")
    {
        return Err(TransferError::new(
            400,
            "Please choose an EPUB or PDF file / 请选择 EPUB 或 PDF 文件",
        ));
    }
    Ok(name)
}

struct InterruptibleStream<'a> {
    stream: &'a mut TcpStream,
    stop: &'a AtomicBool,
    deadline: Instant,
    last_read: Instant,
}
impl Read for InterruptibleStream<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            if self.stop.load(Ordering::Acquire) || Instant::now() >= self.deadline {
                return Err(io::Error::new(
                    io::ErrorKind::ConnectionAborted,
                    "Transfer stopped",
                ));
            }
            match self.stream.read(buf) {
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) && self.last_read.elapsed() < IO_IDLE_TIMEOUT =>
                {
                    continue
                }
                Ok(n) => {
                    self.last_read = Instant::now();
                    return Ok(n);
                }
                result => return result,
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn serve_connection(
    stream: &mut TcpStream,
    token: &str,
    root: &Path,
    page: &str,
    english: bool,
    stop: &AtomicBool,
    deadline: Instant,
    import: &impl Fn(&Path) -> Result<String, String>,
    notice: &impl Fn(TransferNotice),
) -> Result<(), TransferError> {
    stream.set_read_timeout(Some(Duration::from_millis(250)))?;
    stream.set_write_timeout(Some(Duration::from_secs(3)))?;
    // Browsers may open speculative connections without sending a request.
    // Don't let one idle socket stall the next upload for the body timeout.
    match stream.peek(&mut [0u8; 1]) {
        Ok(0) => return Ok(()),
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
            ) =>
        {
            return Ok(())
        }
        Err(error) => return Err(error.into()),
        Ok(_) => {}
    }
    let mut reader = BufReader::new(InterruptibleStream {
        stream,
        stop,
        deadline: deadline.min(Instant::now() + Duration::from_secs(3)),
        last_read: Instant::now(),
    });
    let request = read_request(&mut reader)?;
    reader.get_mut().deadline = deadline;
    let base = format!("/{token}");
    if request.method == "GET" && (request.target == base || request.target == format!("{base}/")) {
        drop(reader);
        respond(stream, 200, "text/html; charset=utf-8", page.as_bytes())?;
        return Ok(());
    }
    let prefix = format!("{base}/upload?name=");
    let Some(name) = request.target.strip_prefix(&prefix) else {
        drop(reader);
        respond(
            stream,
            404,
            "application/json",
            &json_message(false, "This transfer link is unavailable"),
        )?;
        return Ok(());
    };
    if request.method != "POST" {
        return Err(TransferError::new(405, "Use POST to upload"));
    }
    // Browser uploads must originate from this exact receiver. No CORS is enabled.
    if let Some(origin) = request.headers.get("origin") {
        let expected = request.headers.get("host").map(|h| format!("http://{h}"));
        if expected.as_ref() != Some(origin) {
            return Err(TransferError::new(403, "Invalid upload origin"));
        }
    }
    if request.headers.contains_key("transfer-encoding") {
        return Err(TransferError::new(400, "Use a fixed upload length"));
    }
    let size = request
        .headers
        .get("content-length")
        .and_then(|s| s.parse::<u64>().ok())
        .ok_or_else(|| TransferError::new(411, "Missing upload length"))?;
    if size == 0 || size > MAX_UPLOAD_BYTES {
        return Err(TransferError::new(
            413,
            "Books must be between 1 byte and 200 MB / 书籍最大 200 MB",
        ));
    }
    let filename = decoded_filename(name)?;
    let upload_dir = root.join(uuid::Uuid::new_v4().simple().to_string());
    fs::create_dir(&upload_dir)?;
    let _cleanup = TemporaryDirectory(upload_dir.clone());
    let path = upload_dir.join(&filename);
    let mut file = File::create(&path)?;
    notice(TransferNotice::Receiving(filename.clone()));
    let mut remaining = size;
    let mut buffer = [0u8; 64 * 1024];
    while remaining > 0 {
        if stop.load(Ordering::Acquire) || Instant::now() >= deadline {
            return Err(TransferError::new(410, "Transfer stopped"));
        }
        let chunk = remaining.min(buffer.len() as u64) as usize;
        let n = reader.read(&mut buffer[..chunk])?;
        if n == 0 {
            return Err(TransferError::new(400, "Upload interrupted / 文件传输中断"));
        }
        file.write_all(&buffer[..n])?;
        remaining -= n as u64;
    }
    drop(file);
    drop(reader);
    if stop.load(Ordering::Acquire) {
        return Err(TransferError::new(410, "Transfer stopped"));
    }
    match import(&path) {
        Ok(title) => {
            notice(TransferNotice::Imported(title.clone()));
            respond(stream, 201, "application/json", &json_message(true, &title))?;
        }
        Err(error) => {
            let message = if english {
                format!("Could not import {filename}: {error}")
            } else {
                format!("{filename} 导入失败：{error}")
            };
            return Err(TransferError::new(422, message));
        }
    }
    Ok(())
}

fn json_message(ok: bool, message: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({"ok": ok, "message": message})).unwrap()
}

fn respond(stream: &mut TcpStream, status: u16, content_type: &str, body: &[u8]) -> io::Result<()> {
    let reason = match status {
        200 => "OK",
        201 => "Created",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        410 => "Gone",
        411 => "Length Required",
        413 => "Content Too Large",
        422 => "Unprocessable Content",
        431 => "Request Header Fields Too Large",
        _ => "Bad Request",
    };
    write!(stream, "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nX-Content-Type-Options: nosniff\r\nContent-Security-Policy: default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; connect-src 'self'; base-uri 'none'; frame-ancestors 'none'; form-action 'none'\r\n\r\n", body.len())?;
    stream.write_all(body)
}

fn local_addresses() -> Vec<Ipv4Addr> {
    #[cfg(unix)]
    {
        let mut head: *mut libc::ifaddrs = std::ptr::null_mut();
        let mut addresses = Vec::new();
        // getifaddrs owns this linked list until freeifaddrs; check the family
        // before interpreting an address as sockaddr_in.
        unsafe {
            if libc::getifaddrs(&mut head) != 0 {
                return addresses;
            }
            let mut entry = head;
            while !entry.is_null() {
                let item = &*entry;
                if !item.ifa_addr.is_null()
                    && (*item.ifa_addr).sa_family as i32 == libc::AF_INET
                    && item.ifa_flags & libc::IFF_UP as u32 != 0
                {
                    let addr = &*(item.ifa_addr as *const libc::sockaddr_in);
                    let ip = Ipv4Addr::from(addr.sin_addr.s_addr.to_ne_bytes());
                    let name = std::ffi::CStr::from_ptr(item.ifa_name).to_string_lossy();
                    // Android cellular (rmnet/ccmni) and VPN interfaces must
                    // not be offered as Wi-Fi transfer addresses.
                    let lan = name.starts_with("wlan")
                        || name.starts_with("en")
                        || name.starts_with("eth")
                        || name.starts_with("ap")
                        || name.starts_with("bridge");
                    if lan && (ip.is_private() || ip.is_link_local()) && !addresses.contains(&ip) {
                        addresses.push(ip);
                    }
                }
                entry = item.ifa_next;
            }
            libc::freeifaddrs(head);
        }
        addresses
    }
    #[cfg(not(unix))]
    {
        let ip = std::net::UdpSocket::bind("0.0.0.0:0").ok().and_then(|s| {
            s.connect("192.0.2.1:9").ok()?;
            match s.local_addr().ok()?.ip() {
                std::net::IpAddr::V4(ip) if ip.is_private() => Some(ip),
                _ => None,
            }
        });
        ip.into_iter().collect()
    }
}

fn upload_page(english: bool) -> String {
    include_str!("../resources/lan_transfer.html")
        .replace("__LANG__", if english { "en" } else { "zh" })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    fn temporary_root() -> TemporaryDirectory {
        let root = std::env::temp_dir().join(format!("readwise-lan-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        TemporaryDirectory(root)
    }

    fn request(
        receiver: &LanTransfer,
        method: &str,
        target: &str,
        headers: &str,
        body: &[u8],
    ) -> String {
        let url = receiver.urls[0].strip_prefix("http://").unwrap();
        let address = url.split('/').next().unwrap();
        let mut stream = TcpStream::connect(address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        write!(
            stream,
            "{method} {target} HTTP/1.1\r\nHost: {address}\r\n{headers}\r\n"
        )
        .unwrap();
        stream.write_all(body).unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    }

    #[test]
    fn filename_decoding_rejects_traversal_and_preserves_unicode() {
        assert_eq!(
            decoded_filename("%E4%B8%AD%E6%96%87%20book.EPUB").unwrap(),
            "中文 book.EPUB"
        );
        assert_eq!(
            decoded_filename("%E4%B8%AD%E6%96%87.PDF").unwrap(),
            "中文.PDF"
        );
        for name in [
            "../a.pdf",
            "..%2Fa.pdf",
            ".pdf",
            "../a.epub",
            "..%2Fa.epub",
            "%5Ctest.epub",
            "%00a.epub",
            "book.txt",
            ".epub",
            "bad%2.epub",
            "bad%FF.epub",
        ] {
            assert!(decoded_filename(name).is_err(), "{name}");
        }
    }

    #[test]
    fn ambiguous_or_unbounded_headers_are_rejected() {
        let duplicate = b"POST / HTTP/1.1\r\nContent-Length: 3\r\ncontent-length: 8\r\n\r\n";
        assert_eq!(read_request(&mut &duplicate[..]).err().unwrap().status, 400);
        let huge = vec![b'a'; HEADER_LIMIT + 100];
        assert_eq!(read_request(&mut &huge[..]).err().unwrap().status, 431);
    }

    #[test]
    fn real_receiver_handles_batch_binary_uploads_and_import_failures() {
        let root = temporary_root();
        let imported = Arc::new(AtomicUsize::new(0));
        let counter = imported.clone();
        let receiver = LanTransfer::start_on(
            &root.0,
            false,
            vec![Ipv4Addr::LOCALHOST],
            move |path| {
                let bytes = fs::read(path).unwrap();
                assert_eq!(path.file_name().unwrap(), "中文 book.epub");
                if bytes == b"invalid" {
                    return Err("invalid EPUB".into());
                }
                assert_eq!(bytes, b"PK\x03\x04\0\xffbinary");
                counter.fetch_add(1, Ordering::SeqCst);
                Ok("Imported title".into())
            },
            |_, _| {},
        )
        .unwrap();
        let page = request(&receiver, "GET", &format!("/{}", receiver.id), "", b"");
        assert!(page.starts_with("HTTP/1.1 200"));
        assert!(page.contains("type=\"file\""));
        let target = format!(
            "/{}/upload?name=%E4%B8%AD%E6%96%87%20book.epub",
            receiver.id
        );
        for _ in 0..3 {
            let body = b"PK\x03\x04\0\xffbinary";
            let response = request(
                &receiver,
                "POST",
                &target,
                &format!("Content-Length: {}\r\n", body.len()),
                body,
            );
            assert!(response.starts_with("HTTP/1.1 201"), "{response}");
            assert!(response.contains("Imported title"));
        }
        let failed = request(
            &receiver,
            "POST",
            &target,
            "Content-Length: 7\r\n",
            b"invalid",
        );
        assert!(failed.starts_with("HTTP/1.1 422"));
        assert_eq!(imported.load(Ordering::SeqCst), 3);
        assert_eq!(
            fs::read_dir(root.0.join("transfer-cache").join(&receiver.id))
                .unwrap()
                .count(),
            0
        );
    }

    #[test]
    fn pdf_upload_preserves_extension_and_waits_for_import_result() {
        let root = temporary_root();
        let receiver = LanTransfer::start_on(
            &root.0,
            false,
            vec![Ipv4Addr::LOCALHOST],
            |path| {
                assert_eq!(path.file_name().unwrap(), "中文.PDF");
                let bytes = fs::read(path).unwrap();
                if bytes != b"%PDF-1.4\n\0\xff" {
                    return Err("Invalid PDF".into());
                }
                Ok("PDF title".into())
            },
            |_, _| {},
        )
        .unwrap();
        let target = format!("/{}/upload?name=%E4%B8%AD%E6%96%87.PDF", receiver.id);
        let body = b"%PDF-1.4\n\0\xff";
        let response = request(
            &receiver,
            "POST",
            &target,
            &format!("Content-Length: {}\r\n", body.len()),
            body,
        );
        assert!(response.starts_with("HTTP/1.1 201"), "{response}");
        assert!(response.contains("PDF title"));
        let response = request(&receiver, "POST", &target, "Content-Length: 3\r\n", b"bad");
        assert!(response.starts_with("HTTP/1.1 422"), "{response}");
        assert_eq!(
            fs::read_dir(root.0.join("transfer-cache").join(&receiver.id))
                .unwrap()
                .count(),
            0
        );
    }

    #[test]
    fn unauthorized_oversized_and_cross_origin_uploads_never_reach_importer() {
        let root = temporary_root();
        let receiver = LanTransfer::start_on(
            &root.0,
            true,
            vec![Ipv4Addr::LOCALHOST],
            |_| panic!("must not import"),
            |_, _| {},
        )
        .unwrap();
        let target = format!("/{}/upload?name=a.epub", receiver.id);
        let wrong = request(
            &receiver,
            "POST",
            "/wrong/upload?name=a.epub",
            "Content-Length: 1\r\n",
            b"x",
        );
        assert!(wrong.starts_with("HTTP/1.1 404"));
        let oversized = request(
            &receiver,
            "POST",
            &target,
            &format!("Content-Length: {}\r\n", MAX_UPLOAD_BYTES + 1),
            b"",
        );
        assert!(oversized.starts_with("HTTP/1.1 413"));
        let cross_origin = request(
            &receiver,
            "POST",
            &target,
            "Content-Length: 1\r\nOrigin: http://another-site\r\n",
            b"x",
        );
        assert!(cross_origin.starts_with("HTTP/1.1 403"));
        let traversal = request(
            &receiver,
            "POST",
            &format!("/{}/upload?name=..%2Fa.epub", receiver.id),
            "Content-Length: 1\r\n",
            b"x",
        );
        assert!(traversal.starts_with("HTTP/1.1 400"));
    }

    #[test]
    fn closing_session_interrupts_partial_upload_and_cleans_private_files() {
        let root = temporary_root();
        let (events, received) = std::sync::mpsc::channel();
        let receiver = LanTransfer::start_on(
            &root.0,
            false,
            vec![Ipv4Addr::LOCALHOST],
            |_| panic!("must not import partial file"),
            move |_, event| {
                let _ = events.send(event);
            },
        )
        .unwrap();
        let session_root = root.0.join("transfer-cache").join(&receiver.id);
        let address = receiver.urls[0]
            .strip_prefix("http://")
            .unwrap()
            .split('/')
            .next()
            .unwrap()
            .to_owned();
        let mut stream = TcpStream::connect(&address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let partial_request = format!("POST /{}/upload?name=test.epub HTTP/1.1\r\nHost: {address}\r\nContent-Length: 1000\r\n\r\npartial", receiver.id);
        stream.write_all(partial_request.as_bytes()).unwrap();
        assert!(matches!(
            received.recv_timeout(Duration::from_secs(5)).unwrap(),
            TransferNotice::Receiving(_)
        ));
        drop(receiver);
        let mut response = Vec::new();
        stream.read_to_end(&mut response).unwrap();
        assert!(matches!(
            received.recv_timeout(Duration::from_secs(3)).unwrap(),
            TransferNotice::Stopped
        ));
        let deadline = Instant::now() + Duration::from_secs(3);
        while session_root.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(!session_root.exists());
        assert!(TcpStream::connect(address).is_err());
    }

    #[test]
    fn browser_speculative_connection_does_not_block_a_request() {
        let root = temporary_root();
        let receiver = LanTransfer::start_on(
            &root.0,
            true,
            vec![Ipv4Addr::LOCALHOST],
            |_| panic!("must not import"),
            |_, _| {},
        )
        .unwrap();
        let address = receiver.urls[0]
            .strip_prefix("http://")
            .unwrap()
            .split('/')
            .next()
            .unwrap();
        let _idle = TcpStream::connect(address).unwrap();
        assert!(
            request(&receiver, "GET", &format!("/{}", receiver.id), "", b"")
                .starts_with("HTTP/1.1 200")
        );
    }

    #[test]
    fn disconnected_upload_is_not_imported_and_receiver_remains_usable() {
        let root = temporary_root();
        let receiver = LanTransfer::start_on(
            &root.0,
            true,
            vec![Ipv4Addr::LOCALHOST],
            |_| panic!("must not import"),
            |_, _| {},
        )
        .unwrap();
        let address = receiver.urls[0]
            .strip_prefix("http://")
            .unwrap()
            .split('/')
            .next()
            .unwrap();
        let mut stream = TcpStream::connect(address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let partial_request = format!("POST /{}/upload?name=test.epub HTTP/1.1\r\nHost: {address}\r\nContent-Length: 1000\r\n\r\npartial", receiver.id);
        stream.write_all(partial_request.as_bytes()).unwrap();
        stream.shutdown(std::net::Shutdown::Write).unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(
            response.starts_with("HTTP/1.1 400"),
            "response: {response:?}"
        );
        assert!(
            request(&receiver, "GET", &format!("/{}", receiver.id), "", b"")
                .starts_with("HTTP/1.1 200")
        );
        assert_eq!(
            fs::read_dir(root.0.join("transfer-cache").join(&receiver.id))
                .unwrap()
                .count(),
            0
        );
    }
}
