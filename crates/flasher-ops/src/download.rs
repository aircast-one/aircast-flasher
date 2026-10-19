use super::filename_from_url;
use super::http;
use super::parse_sha256;
use super::Arc;
use super::AtomicBool;
use super::DownloadProgress;
use super::DownloadResult;
use super::Duration;
use super::Ordering;
use super::Sha256;
use super::BACKOFF_BASE;
use super::BACKOFF_CAP;
use super::CANCEL_POLL;
use super::MAX_RETRIES;
use super::PROGRESS_THROTTLE_MS;
use super::SPEED_WINDOW;
use super::STALLED_ATTEMPTS;
use futures_util::StreamExt as _;
use sha2::Digest as _;
use std::io::Read as _;
use tokio::io::AsyncSeekExt as _;
use tokio::io::AsyncWriteExt as _;

async fn fetch_checksum(checksum_url: &str) -> Result<String, String> {
    let raw = http()
        .get(checksum_url)
        .send()
        .await
        .map_err(|e| format!("Failed to fetch checksum: {e}"))?
        .text()
        .await
        .map_err(|e| format!("Failed to read checksum: {e}"))?;
    parse_sha256(raw.split_whitespace().next().unwrap_or(""))
}

fn reuse_cached(image_path: &std::path::Path, checksum: &str) -> Result<bool, String> {
    if marker_validates(image_path, checksum) {
        return Ok(true);
    }
    if verify_file_checksum(image_path, checksum)? {
        if let Ok(meta) = std::fs::metadata(image_path) {
            write_marker(image_path, checksum, meta.len());
        }
        return Ok(true);
    }
    let _ = std::fs::remove_file(image_path);
    let _ = std::fs::remove_file(marker_path(image_path));
    let _ = std::fs::remove_file(part_path(image_path));
    Ok(false)
}

async fn stream_with_retries(
    emit: &mut (dyn FnMut(&DownloadProgress) + Send),
    cancel: &Arc<AtomicBool>,
    download_url: &str,
    sink: &mut Sink,
    stats: &mut DownloadStats,
) -> Result<(), String> {
    let mut stalled: usize = 0;
    loop {
        let attempt_started = std::time::Instant::now();
        let before = sink.written;
        let result = stream_into(emit, cancel, download_url, sink, stats).await;
        stats.transfer_ms += attempt_started.elapsed().as_millis() as u64;

        match result {
            Ok(()) => return Ok(()),
            Err(Fail::Fatal(msg)) => return Err(msg),
            Err(Fail::Transient(msg)) => {
                stalled = if sink.written > before {
                    0
                } else {
                    stalled + 1
                };
                if stalled >= STALLED_ATTEMPTS || stats.retries >= MAX_RETRIES {
                    return Err(msg);
                }
                stats.retries += 1;
                emit(&DownloadProgress {
                    downloaded_bytes: sink.written,
                    total_bytes: sink.total,
                    percent: percent_of(sink.written, sink.total),
                    speed_bps: 0,
                });
                if !sleep_unless_cancelled(cancel, backoff(stalled)).await {
                    return Err("Download cancelled".to_string());
                }
            }
        }
    }
}

pub async fn download_image(
    cache_dir: &std::path::Path,
    download_url: &str,
    checksum_url: &str,
    cancel: &Arc<AtomicBool>,
    progress: &mut (dyn FnMut(&DownloadProgress) + Send),
    stats: &mut DownloadStats,
) -> Result<DownloadResult, String> {
    let checksum = fetch_checksum(checksum_url).await?;

    std::fs::create_dir_all(cache_dir).map_err(|e| format!("Failed to create cache dir: {e}"))?;

    let filename = filename_from_url(download_url);
    let image_path = cache_dir.join(&filename);

    let resumed = resume_partial(&image_path, &checksum).await;

    if resumed.is_none() && image_path.exists() && reuse_cached(&image_path, &checksum)? {
        return Ok(DownloadResult {
            image_path: image_path.to_string_lossy().to_string(),
            checksum,
            cached: true,
        });
    }

    let mut sink = match resumed {
        Some(sink) => sink,
        None => Sink::create(&image_path).await?,
    };

    let outcome = stream_with_retries(progress, cancel, download_url, &mut sink, stats).await;

    if let Err(msg) = outcome {
        let _ = sink.file.flush().await;
        if cancel.load(Ordering::SeqCst) {
            let _ = std::fs::remove_file(&image_path);
            let _ = std::fs::remove_file(part_path(&image_path));
        } else if sink.written > 0 {
            let _ = std::fs::write(
                part_path(&image_path),
                format!("{checksum} {}", sink.written),
            );
        }
        return Err(msg);
    }

    sink.file
        .flush()
        .await
        .map_err(|e| format!("Failed to flush cache file: {e}"))?;

    let downloaded_bytes = sink.written;
    let total_bytes = sink.total;
    let hasher = sink.hasher;

    progress(&DownloadProgress {
        downloaded_bytes,
        total_bytes,
        percent: 100.0,
        speed_bps: 0,
    });

    let actual = format!("{:x}", hasher.finalize());
    if actual != checksum.to_ascii_lowercase() {
        let _ = std::fs::remove_file(&image_path);
        let _ = std::fs::remove_file(part_path(&image_path));
        return Err(format!(
            "Checksum mismatch after download (expected {checksum}, got {actual})"
        ));
    }
    let _ = std::fs::remove_file(part_path(&image_path));
    write_marker(&image_path, &checksum, downloaded_bytes);

    Ok(DownloadResult {
        image_path: image_path.to_string_lossy().to_string(),
        checksum,
        cached: false,
    })
}

struct Sink {
    file: tokio::fs::File,
    hasher: Sha256,
    written: u64,
    total: u64,
}

impl Sink {
    async fn create(image_path: &std::path::Path) -> Result<Self, String> {
        Ok(Self {
            file: tokio::fs::File::create(image_path)
                .await
                .map_err(|e| format!("Failed to create cache file: {e}"))?,
            hasher: Sha256::new(),
            written: 0,
            total: 0,
        })
    }
}

#[derive(Default)]
pub struct DownloadStats {
    pub retries: usize,
    pub restarts: usize,
    pub resumed_bytes: u64,
    pub transfer_ms: u64,
}

enum Fail {
    Transient(String),
    Fatal(String),
}

async fn resume_partial(image_path: &std::path::Path, checksum: &str) -> Option<Sink> {
    let (sha, bytes) = read_marker_file(&part_path(image_path))?;
    if bytes == 0 || !sha.eq_ignore_ascii_case(checksum) {
        return None;
    }
    if std::fs::metadata(image_path).ok()?.len() < bytes {
        return None;
    }

    let path = image_path.to_path_buf();
    let hasher = tokio::task::spawn_blocking(move || hash_prefix(&path, bytes))
        .await
        .ok()?
        .ok()?;

    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .open(image_path)
        .await
        .ok()?;
    file.set_len(bytes).await.ok()?;
    file.seek(std::io::SeekFrom::Start(bytes)).await.ok()?;

    Some(Sink {
        file,
        hasher,
        written: bytes,
        total: 0,
    })
}

fn hash_prefix(path: &std::path::Path, bytes: u64) -> std::io::Result<Sha256> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1024 * 1024];
    let mut left = bytes;
    while left > 0 {
        let want = buf.len().min(left as usize);
        let n = file.read(&mut buf[..want])?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        left -= n as u64;
    }
    Ok(hasher)
}

fn percent_of(done: u64, total: u64) -> f64 {
    if total > 0 {
        ((done as f64 / total as f64) * 100.0).min(100.0)
    } else {
        0.0
    }
}

fn backoff(stalled: usize) -> Duration {
    BACKOFF_CAP.min(BACKOFF_BASE * 2u32.saturating_pow(stalled.saturating_sub(1) as u32))
}

async fn sleep_unless_cancelled(cancel: &Arc<AtomicBool>, total: Duration) -> bool {
    let deadline = std::time::Instant::now() + total;
    while let Some(left) = deadline.checked_duration_since(std::time::Instant::now()) {
        if cancel.load(Ordering::SeqCst) {
            return false;
        }
        tokio::time::sleep(CANCEL_POLL.min(left)).await;
    }
    !cancel.load(Ordering::SeqCst)
}

fn content_range_start(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    headers
        .get(reqwest::header::CONTENT_RANGE)?
        .to_str()
        .ok()?
        .trim()
        .strip_prefix("bytes ")?
        .split('-')
        .next()?
        .trim()
        .parse()
        .ok()
}

async fn stream_into(
    progress: &mut (dyn FnMut(&DownloadProgress) + Send),
    cancel: &Arc<AtomicBool>,
    url: &str,
    sink: &mut Sink,
    stats: &mut DownloadStats,
) -> Result<(), Fail> {
    if cancel.load(Ordering::SeqCst) {
        return Err(Fail::Fatal("Download cancelled".to_string()));
    }

    let request = match sink.written {
        0 => http().get(url),
        n => http()
            .get(url)
            .header(reqwest::header::RANGE, format!("bytes={n}-")),
    };

    let response = request
        .send()
        .await
        .map_err(|e| Fail::Transient(format!("Download error: {e}")))?;

    if !response.status().is_success() {
        let status = response.status();
        let msg = format!("Download failed: HTTP {status}");
        return Err(if status.is_server_error() {
            Fail::Transient(msg)
        } else {
            Fail::Fatal(msg)
        });
    }

    let starts_at = if response.status() == reqwest::StatusCode::PARTIAL_CONTENT {
        content_range_start(response.headers())
    } else {
        Some(0)
    };

    if starts_at != Some(sink.written) {
        sink.file
            .set_len(0)
            .await
            .map_err(|e| Fail::Fatal(format!("Failed to restart download: {e}")))?;
        sink.file
            .seek(std::io::SeekFrom::Start(0))
            .await
            .map_err(|e| Fail::Fatal(format!("Failed to restart download: {e}")))?;
        sink.hasher = Sha256::new();
        sink.written = 0;
        stats.restarts += 1;
    } else if sink.written > 0 {
        stats.resumed_bytes += sink.written;
    }

    if sink.total == 0 {
        sink.total = response.content_length().unwrap_or(0) + sink.written;
    }

    let mut stream = response.bytes_stream();
    let mut last_emit = std::time::Instant::now();
    let mut window_start = std::time::Instant::now();
    let mut window_bytes: u64 = 0;
    let mut speed_bps: u64 = 0;

    while let Some(chunk_result) = stream.next().await {
        if cancel.load(Ordering::SeqCst) {
            return Err(Fail::Fatal("Download cancelled".to_string()));
        }
        let chunk = chunk_result.map_err(|e| Fail::Transient(format!("Download error: {e}")))?;
        sink.file
            .write_all(&chunk)
            .await
            .map_err(|e| Fail::Fatal(format!("Failed to write chunk: {e}")))?;
        sink.hasher.update(&chunk);
        sink.written += chunk.len() as u64;
        window_bytes += chunk.len() as u64;

        if window_start.elapsed() >= SPEED_WINDOW {
            speed_bps = (window_bytes as f64 / window_start.elapsed().as_secs_f64()) as u64;
            window_start = std::time::Instant::now();
            window_bytes = 0;
        }

        if last_emit.elapsed().as_millis() >= PROGRESS_THROTTLE_MS {
            progress(&DownloadProgress {
                downloaded_bytes: sink.written,
                total_bytes: sink.total,
                percent: percent_of(sink.written, sink.total),
                speed_bps,
            });
            last_emit = std::time::Instant::now();
        }
    }

    if sink.total > 0 && sink.written < sink.total {
        return Err(Fail::Transient(format!(
            "Download truncated at {} of {} bytes",
            sink.written, sink.total
        )));
    }

    Ok(())
}

fn verify_file_checksum(path: &std::path::Path, expected: &str) -> Result<bool, String> {
    let mut file =
        std::fs::File::open(path).map_err(|e| format!("Failed to open file for checksum: {e}"))?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher).map_err(|e| format!("Failed to hash file: {e}"))?;
    let hash = format!("{:x}", hasher.finalize());
    Ok(hash == expected.to_ascii_lowercase())
}

fn sidecar(image_path: &std::path::Path, suffix: &str) -> std::path::PathBuf {
    let mut name = image_path.as_os_str().to_owned();
    name.push(suffix);
    std::path::PathBuf::from(name)
}

fn marker_path(image_path: &std::path::Path) -> std::path::PathBuf {
    sidecar(image_path, ".cache")
}

fn part_path(image_path: &std::path::Path) -> std::path::PathBuf {
    sidecar(image_path, ".part")
}

fn read_marker_file(path: &std::path::Path) -> Option<(String, u64)> {
    let content = std::fs::read_to_string(path).ok()?;
    let mut parts = content.split_whitespace();
    let sha = parts.next()?.to_string();
    let size = parts.next()?.parse::<u64>().ok()?;
    Some((sha, size))
}

fn read_marker(image_path: &std::path::Path) -> Option<(String, u64)> {
    read_marker_file(&marker_path(image_path))
}

fn write_marker(image_path: &std::path::Path, checksum: &str, size: u64) {
    let _ = std::fs::write(marker_path(image_path), format!("{checksum} {size}"));
}

fn marker_validates(image_path: &std::path::Path, expected: &str) -> bool {
    match (read_marker(image_path), std::fs::metadata(image_path)) {
        (Some((sha, size)), Ok(meta)) => sha.eq_ignore_ascii_case(expected) && meta.len() == size,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::isolated_dir;

    async fn read_request(sock: &mut tokio::net::TcpStream) -> String {
        use tokio::io::AsyncReadExt;

        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            if sock.read(&mut byte).await.unwrap() == 0 {
                break;
            }
            head.push(byte[0]);
        }
        String::from_utf8_lossy(&head).into_owned()
    }

    async fn flaky_server(body: &'static [u8]) -> (String, tokio::task::JoinHandle<bool>) {
        use tokio::io::AsyncWriteExt;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/image", listener.local_addr().unwrap());
        let half = body.len() / 2;

        let handle = tokio::spawn(async move {
            let (mut first, _) = listener.accept().await.unwrap();
            read_request(&mut first).await;
            first
                .write_all(
                    format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len()).as_bytes(),
                )
                .await
                .unwrap();
            first.write_all(&body[..half]).await.unwrap();
            first.flush().await.unwrap();
            drop(first);

            let (mut second, _) = listener.accept().await.unwrap();
            let asked_for_resume = read_request(&mut second)
                .await
                .contains(&format!("bytes={half}-"));
            second
                    .write_all(
                        format!(
                            "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {}-{}/{}\r\n\r\n",
                            body.len() - half,
                            half,
                            body.len() - 1,
                            body.len()
                        )
                        .as_bytes(),
                    )
                    .await
                    .unwrap();
            second.write_all(&body[half..]).await.unwrap();
            second.flush().await.unwrap();
            asked_for_resume
        });

        (url, handle)
    }

    async fn hostile_resume_server(
        body: &'static [u8],
        status: &'static str,
        content_range: Option<&'static str>,
    ) -> (String, tokio::task::JoinHandle<()>) {
        use tokio::io::AsyncWriteExt;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/image", listener.local_addr().unwrap());
        let half = body.len() / 2;

        let handle = tokio::spawn(async move {
            let (mut first, _) = listener.accept().await.unwrap();
            read_request(&mut first).await;
            first
                .write_all(
                    format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len()).as_bytes(),
                )
                .await
                .unwrap();
            first.write_all(&body[..half]).await.unwrap();
            first.flush().await.unwrap();
            drop(first);

            let (mut second, _) = listener.accept().await.unwrap();
            read_request(&mut second).await;
            let range = content_range
                .map(|r| format!("Content-Range: {r}\r\n"))
                .unwrap_or_default();
            second
                .write_all(
                    format!(
                        "HTTP/1.1 {status}\r\nContent-Length: {}\r\n{range}\r\n",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            second.write_all(body).await.unwrap();
            second.flush().await.unwrap();
        });

        (url, handle)
    }

    async fn test_sink(name: &str) -> (std::path::PathBuf, Sink) {
        let path = isolated_dir(name).join("image.img.xz");
        let sink = Sink {
            file: tokio::fs::File::create(&path).await.unwrap(),
            hasher: Sha256::new(),
            written: 0,
            total: 0,
        };
        (path, sink)
    }

    async fn finish(sink: &mut Sink) -> Vec<u8> {
        sink.file.flush().await.unwrap();
        sink.hasher.clone().finalize().to_vec()
    }

    #[tokio::test]
    async fn download_resumes_after_the_connection_drops() {
        const BODY: &[u8] = b"aircast-image-payload-that-spans-two-http-responses";

        let (url, server) = flaky_server(BODY).await;
        let (path, mut sink) = test_sink("resume").await;
        let cancel = Arc::new(AtomicBool::new(false));
        let mut stats = DownloadStats::default();

        let first = stream_into(&mut |_| {}, &cancel, &url, &mut sink, &mut stats).await;
        assert!(
            matches!(first, Err(Fail::Transient(_))),
            "a mid-stream drop must be retryable, not fatal"
        );
        assert!(sink.written > 0 && sink.written < BODY.len() as u64);

        stream_into(&mut |_| {}, &cancel, &url, &mut sink, &mut stats)
            .await
            .map_err(|e| match e {
                Fail::Transient(m) | Fail::Fatal(m) => m,
            })
            .expect("the retry must complete the download");

        assert!(server.await.unwrap(), "the retry must send a Range header");
        assert_eq!(sink.written, BODY.len() as u64);
        assert_eq!(
            finish(&mut sink).await,
            Sha256::digest(BODY).to_vec(),
            "the hash must span both halves exactly once"
        );
        assert_eq!(std::fs::read(&path).unwrap(), BODY, "bytes on disk");
        assert_eq!(stats.restarts, 0, "a clean resume must not restart");
        assert!(stats.resumed_bytes > 0, "resumed bytes must be recorded");
    }

    #[tokio::test]
    async fn download_restarts_when_the_server_resumes_at_the_wrong_offset() {
        const BODY: &[u8] = b"aircast-image-payload-that-spans-two-http-responses";

        for (name, status, range) in [
            ("wrong-offset", "206 Partial Content", Some("bytes 0-49/50")),
            ("no-range", "206 Partial Content", None),
            ("ignores-range", "200 OK", None),
        ] {
            let (url, server) = hostile_resume_server(BODY, status, range).await;
            let (path, mut sink) = test_sink(name).await;
            let cancel = Arc::new(AtomicBool::new(false));
            let mut stats = DownloadStats::default();

            let _ = stream_into(&mut |_| {}, &cancel, &url, &mut sink, &mut stats).await;
            assert!(sink.written > 0 && sink.written < BODY.len() as u64);

            stream_into(&mut |_| {}, &cancel, &url, &mut sink, &mut stats)
                .await
                .map_err(|e| match e {
                    Fail::Transient(m) | Fail::Fatal(m) => m,
                })
                .unwrap_or_else(|e| panic!("{name}: restart must complete: {e}"));

            server.await.unwrap();
            assert_eq!(stats.restarts, 1, "{name}: must restart exactly once");
            assert_eq!(sink.written, BODY.len() as u64, "{name}: length");
            assert_eq!(
                finish(&mut sink).await,
                Sha256::digest(BODY).to_vec(),
                "{name}: the hash must cover the restarted stream only"
            );
            assert_eq!(std::fs::read(&path).unwrap(), BODY, "{name}: bytes on disk");
        }
    }

    #[tokio::test]
    async fn cancelling_skips_the_request_and_cuts_the_backoff_short() {
        let cancel = Arc::new(AtomicBool::new(true));
        let (_path, mut sink) = test_sink("cancel").await;
        let mut stats = DownloadStats::default();

        let started = std::time::Instant::now();
        let result = stream_into(
            &mut |_| {},
            &cancel,
            "http://127.0.0.1:1/never-listens",
            &mut sink,
            &mut stats,
        )
        .await;
        assert!(
            matches!(result, Err(Fail::Fatal(ref m)) if m.contains("cancelled")),
            "a cancelled download must not issue the request at all"
        );

        assert!(
            !sleep_unless_cancelled(&cancel, Duration::from_secs(30)).await,
            "backoff must report the cancellation"
        );
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "cancel must not wait out the connect timeout or the backoff"
        );
    }

    #[test]
    fn content_range_start_reads_the_offset() {
        let parse = |v: &str| {
            let mut h = reqwest::header::HeaderMap::new();
            h.insert(reqwest::header::CONTENT_RANGE, v.parse().unwrap());
            content_range_start(&h)
        };
        assert_eq!(parse("bytes 1024-2047/2048"), Some(1024));
        assert_eq!(parse("bytes 0-0/1"), Some(0));
        assert_eq!(parse("items 1024-2047/2048"), None);
        assert_eq!(parse("bytes */2048"), None);
        assert_eq!(
            content_range_start(&reqwest::header::HeaderMap::new()),
            None
        );
    }

    #[test]
    fn backoff_grows_then_caps() {
        assert_eq!(backoff(1), BACKOFF_BASE);
        assert_eq!(backoff(2), BACKOFF_BASE * 2);
        assert_eq!(backoff(3), BACKOFF_BASE * 4);
        assert_eq!(backoff(99), BACKOFF_CAP, "must not overflow or run away");
    }

    #[tokio::test]
    async fn a_failed_download_resumes_from_its_partial_file() {
        const BODY: &[u8] = b"aircast-image-payload-that-spans-two-http-responses";
        let half = BODY.len() / 2;
        let checksum = format!("{:x}", Sha256::digest(BODY));

        let dir = isolated_dir("partial");
        let image = dir.join("image.img.xz");
        std::fs::write(&image, &BODY[..half]).unwrap();
        std::fs::write(part_path(&image), format!("{checksum} {half}")).unwrap();

        let mut sink = resume_partial(&image, &checksum)
            .await
            .expect("a partial from this release must be resumable");
        assert_eq!(sink.written, half as u64);

        sink.hasher.update(&BODY[half..]);
        assert_eq!(
            format!("{:x}", sink.hasher.clone().finalize()),
            checksum,
            "the reseeded hash must match hashing the whole body once"
        );

        assert!(
            resume_partial(&image, &"0".repeat(64)).await.is_none(),
            "a partial from a different release must not be resumed"
        );
    }

    #[test]
    fn percent_never_exceeds_one_hundred() {
        assert_eq!(percent_of(50, 100), 50.0);
        assert_eq!(percent_of(0, 0), 0.0);
        assert_eq!(
            percent_of(120, 100),
            100.0,
            "an over-long body must not render as 120%"
        );
    }

    #[test]
    fn marker_validates_trusts_matching_checksum_and_size() {
        let dir = std::env::temp_dir().join(format!("aircast-cache-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let img = dir.join("image.img.xz");
        std::fs::write(&img, b"hello world").unwrap();
        let checksum = "a".repeat(64);

        assert!(!marker_validates(&img, &checksum));

        write_marker(&img, &checksum, 11);
        assert!(marker_validates(&img, &checksum));
        assert!(marker_validates(&img, &checksum.to_ascii_uppercase()));
        assert!(!marker_validates(&img, &"b".repeat(64)));

        std::fs::write(&img, b"hello").unwrap();
        assert!(!marker_validates(&img, &checksum));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn filename_from_url_strips_query() {
        assert_eq!(
            filename_from_url("https://downloads.aircast.one/images/v1/aircast.img.xz?t=1"),
            "aircast.img.xz"
        );
        assert_eq!(filename_from_url("not-a-url"), "image.img.xz");
    }
}
