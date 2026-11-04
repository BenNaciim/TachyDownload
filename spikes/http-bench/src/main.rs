//! THROWAWAY (proof S2). One download of the bench server's file in `--conns` parallel `Range` requests over HTTP/1.1,
//! written to a preallocated `.part` file, then: throughput, CPU (user / kernel), peak memory, and how many TCP
//! connections the server saw (the claim to check: N parallel ranges = N connections, not one multiplexed HTTP/2 stream).
//!
//!   spike-http-bench --url http://127.0.0.1:PORT/big.bin --conns 8 [--block-kib 1024] [--sparse] [--no-prealloc]
//!
//! The matching server is `node tools/bench/server.js`.

use std::fs::OpenOptions;
use std::os::windows::fs::FileExt;
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use windows::Win32::Foundation::{FILETIME, HANDLE};
use windows::Win32::System::IO::DeviceIoControl;
use windows::Win32::System::Ioctl::FSCTL_SET_SPARSE;
use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

struct Opts {
    url: String,
    conns: u64,
    block: usize,
    sparse: bool,
    prealloc: bool,
}

fn parse() -> Opts {
    let mut o = Opts {
        url: String::new(),
        conns: 8,
        block: 1024 * 1024,
        sparse: false,
        prealloc: true,
    };
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--url" => o.url = args.next().expect("--url needs a value"),
            "--conns" => {
                o.conns = args
                    .next()
                    .and_then(|v| v.parse().ok())
                    .expect("--conns needs a number")
            }
            "--block-kib" => {
                o.block = args
                    .next()
                    .and_then(|v| v.parse::<usize>().ok())
                    .expect("--block-kib needs a number")
                    * 1024
            }
            "--sparse" => o.sparse = true,
            "--no-prealloc" => o.prealloc = false,
            other => panic!("unknown argument {other}"),
        }
    }
    assert!(!o.url.is_empty(), "--url is required");
    o
}

fn filetime_secs(t: FILETIME) -> f64 {
    (((t.dwHighDateTime as u64) << 32) | t.dwLowDateTime as u64) as f64 / 1e7
}

/// (user seconds, kernel seconds) of this process so far.
fn cpu_times() -> (f64, f64) {
    let (mut c, mut e, mut k, mut u) = (
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
    );
    unsafe {
        GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u)
            .expect("GetProcessTimes")
    };
    (filetime_secs(u), filetime_secs(k))
}

fn peak_working_set_mib() -> f64 {
    let mut m = PROCESS_MEMORY_COUNTERS::default();
    m.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
    unsafe {
        GetProcessMemoryInfo(GetCurrentProcess(), &mut m, m.cb).expect("GetProcessMemoryInfo")
    };
    m.PeakWorkingSetSize as f64 / 1048576.0
}

fn mark_sparse(file: &std::fs::File) {
    let mut returned = 0u32;
    unsafe {
        DeviceIoControl(
            HANDLE(file.as_raw_handle()),
            FSCTL_SET_SPARSE,
            None,
            0,
            None,
            0,
            Some(&mut returned),
            None,
        )
        .expect("FSCTL_SET_SPARSE");
    }
}

fn write_all_at(file: &std::fs::File, mut buf: &[u8], mut pos: u64) -> std::io::Result<()> {
    while !buf.is_empty() {
        let n = file.seek_write(buf, pos)?;
        buf = &buf[n..];
        pos += n as u64;
    }
    Ok(())
}

async fn flush(file: &Arc<std::fs::File>, buf: &mut Vec<u8>, pos: &mut u64, block: usize) {
    let data = std::mem::replace(buf, Vec::with_capacity(block));
    let len = data.len() as u64;
    let f = Arc::clone(file);
    let at = *pos;
    tokio::task::spawn_blocking(move || write_all_at(&f, &data, at))
        .await
        .expect("join")
        .expect("disk write");
    *pos += len;
}

async fn segment(
    client: reqwest::Client,
    url: String,
    start: u64,
    end: u64,
    part: PathBuf,
    block: usize,
    got: Arc<AtomicU64>,
) {
    let resp = client
        .get(&url)
        .header("Range", format!("bytes={start}-{end}"))
        .send()
        .await
        .expect("range request");
    assert_eq!(resp.status().as_u16(), 206, "the server must answer 206");
    // One handle per segment: independent cursors, no sharing.
    let file = Arc::new(
        OpenOptions::new()
            .write(true)
            .open(&part)
            .expect("open .part"),
    );
    let mut stream = resp.bytes_stream();
    let mut buf = Vec::with_capacity(block);
    let mut pos = start;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.expect("body");
        got.fetch_add(chunk.len() as u64, Ordering::Relaxed);
        buf.extend_from_slice(&chunk);
        if buf.len() >= block {
            flush(&file, &mut buf, &mut pos, block).await;
        }
    }
    if !buf.is_empty() {
        flush(&file, &mut buf, &mut pos, block).await;
    }
    assert_eq!(pos, end + 1, "segment {start}-{end} ended early");
}

async fn stats(client: &reqwest::Client, origin: &str) -> (u64, u64) {
    let v: serde_json::Value = serde_json::from_str(
        &client
            .get(format!("{origin}/__stats"))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap(),
    )
    .unwrap();
    (
        v["connections"].as_u64().unwrap(),
        v["requests"].as_u64().unwrap(),
    )
}

fn check(path: &Path, size: u64) {
    let f = std::fs::File::open(path).expect("open result");
    assert_eq!(f.metadata().unwrap().len(), size, "wrong size");
    let mut b = [0u8; 1];
    for i in 0..2000u64 {
        let at = (i * 7_919_257) % size; // spread over the whole file
        f.seek_read(&mut b, at).unwrap();
        assert_eq!(b[0], 0x5a, "wrong byte at {at}");
    }
}

#[tokio::main]
async fn main() {
    let o = parse();
    let origin = o.url.split('/').take(3).collect::<Vec<_>>().join("/");
    let client = reqwest::Client::builder()
        .http1_only()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .tcp_nodelay(true)
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(Duration::from_secs(60))
        .build()
        .expect("client");
    let stats_client = reqwest::Client::builder()
        .http1_only()
        .no_proxy()
        .build()
        .unwrap();
    let (conns0, reqs0) = stats(&stats_client, &origin).await;

    let head = client.head(&o.url).send().await.expect("HEAD");
    let size: u64 = head.headers()["content-length"]
        .to_str()
        .unwrap()
        .parse()
        .unwrap();

    let dir = std::env::temp_dir().join(format!("spike-http-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let part = dir.join("big.bin.part");
    {
        let f = std::fs::File::create(&part).unwrap();
        if o.sparse {
            mark_sparse(&f);
        }
        if o.prealloc {
            f.set_len(size).unwrap();
        }
    }

    let got = Arc::new(AtomicU64::new(0));
    let (cpu_u0, cpu_k0) = cpu_times();
    let t0 = Instant::now();
    let samples = Arc::new(std::sync::Mutex::new(Vec::<(f64, u64)>::new()));
    let sampler = {
        let (got, samples) = (got.clone(), samples.clone());
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(100)).await;
                samples
                    .lock()
                    .unwrap()
                    .push((t0.elapsed().as_secs_f64(), got.load(Ordering::Relaxed)));
            }
        })
    };

    let per = size.div_ceil(o.conns);
    let mut tasks = Vec::new();
    for i in 0..o.conns {
        let (s, e) = (i * per, ((i + 1) * per).min(size) - 1);
        tasks.push(tokio::spawn(segment(
            client.clone(),
            o.url.clone(),
            s,
            e,
            part.clone(),
            o.block,
            got.clone(),
        )));
    }
    for t in tasks {
        t.await.expect("segment task");
    }
    let wall = t0.elapsed().as_secs_f64();
    sampler.abort();
    let (cpu_u1, cpu_k1) = cpu_times();

    let s = samples.lock().unwrap();
    let a = s.iter().find(|(_, b)| *b >= size / 10);
    let z = s.iter().find(|(_, b)| *b >= size / 10 * 9);
    let steady = match (a, z) {
        (Some(a), Some(z)) if z.0 > a.0 => (z.1 - a.1) as f64 / (z.0 - a.0) / 1e6,
        _ => f64::NAN,
    };
    let (conns1, reqs1) = stats(&stats_client, &origin).await;
    check(&part, size);
    println!(
        "conns={} block={}KiB sparse={} prealloc={} | wall={:.2}s avg={:.0}MB/s steady={:.0}MB/s | cpu user={:.2}s kernel={:.2}s total={:.2}s ({:.0}% of a core) | peakWS={:.0}MiB | server saw {} TCP connections for {} requests",
        o.conns,
        o.block / 1024,
        o.sparse,
        o.prealloc,
        wall,
        size as f64 / wall / 1e6,
        steady,
        cpu_u1 - cpu_u0,
        cpu_k1 - cpu_k0,
        (cpu_u1 - cpu_u0) + (cpu_k1 - cpu_k0),
        ((cpu_u1 - cpu_u0) + (cpu_k1 - cpu_k0)) / wall * 100.0,
        peak_working_set_mib(),
        conns1 - conns0,
        reqs1 - reqs0,
    );
    std::fs::remove_dir_all(&dir).ok();
}
