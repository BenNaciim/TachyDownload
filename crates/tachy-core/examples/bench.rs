//! Throughput bench of the real Rust core against the local file server of `tools/bench/server.js` (the same one
//! `tools/bench/bench-js.js` uses for the Node engine), so both numbers come from the same origin.
//!
//!   cargo run --release -p tachy-core --example bench
//!   SIZE_MB=256 CONNS=8 cargo run --release -p tachy-core --example bench
//!
//! The origin runs in its own process, so its CPU is not counted against the client.

use serde_json::json;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use tachy_core::manager::{AddOpts, Manager, ManagerConfig};
use tachy_core::state::Status;
use windows::Win32::Foundation::FILETIME;
use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

fn secs(t: FILETIME) -> f64 {
    (((t.dwHighDateTime as u64) << 32) | t.dwLowDateTime as u64) as f64 / 1e7
}

/// (user, kernel) CPU seconds of this process so far.
fn cpu() -> (f64, f64) {
    let (mut c, mut e, mut k, mut u) = (
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
    );
    unsafe { GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u).unwrap() };
    (secs(u), secs(k))
}

fn peak_mib() -> f64 {
    let mut m = PROCESS_MEMORY_COUNTERS {
        cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        ..Default::default()
    };
    unsafe { GetProcessMemoryInfo(GetCurrentProcess(), &mut m, m.cb).unwrap() };
    m.PeakWorkingSetSize as f64 / 1048576.0
}

#[tokio::main]
async fn main() {
    let size_mb: u64 = std::env::var("SIZE_MB")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1024);
    let size = size_mb * 1024 * 1024;
    let conns: Vec<u32> = std::env::var("CONNS")
        .unwrap_or_else(|_| "1,8,16".into())
        .split(',')
        .map(|c| c.trim().parse().unwrap())
        .collect();

    let server_js =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/bench/server.js");
    let mut srv = Command::new("node")
        .arg(&server_js)
        .env("SIZE_MB", size_mb.to_string())
        .stdout(Stdio::piped())
        .spawn()
        .expect("node tools/bench/server.js");
    let port: u16 = {
        let mut lines = BufReader::new(srv.stdout.take().unwrap()).lines();
        loop {
            let line = lines.next().expect("server output").unwrap();
            if let Some(p) = line.strip_prefix("PORT ") {
                break p.trim().parse().unwrap();
            }
        }
    };

    let base = std::env::temp_dir().join(format!("tachy-bench-rs-{}", std::process::id()));
    println!(
        "{:>5} | {:>9} | {:>8} | {:>14} | {:>15} | {:>9} | {:>9} | {:>11}",
        "conns",
        "statut",
        "durée s",
        "débit moy Mo/s",
        "débit stable Mo/s",
        "CPU user",
        "CPU noyau",
        "% d'un cœur"
    );
    for c in conns {
        let dir = base.join(format!("run-{c}"));
        std::fs::create_dir_all(&dir).unwrap();
        let m = Manager::new(ManagerConfig {
            settings: json!({ "dir": dir.to_string_lossy(), "connections": c }),
            tick_ms: 100,
            ..ManagerConfig::default()
        });
        m.start();
        let (u0, k0) = cpu();
        let t0 = Instant::now();
        let id = m
            .add(
                &format!("http://127.0.0.1:{port}/big.bin"),
                &AddOpts {
                    filename: Some(format!("big-{c}.bin")),
                    ..AddOpts::default()
                },
            )
            .unwrap()
            .state
            .id;
        let mut samples: Vec<(f64, u64)> = vec![];
        let status = loop {
            tokio::time::sleep(Duration::from_millis(100)).await;
            let t = m.list().into_iter().find(|t| t.state.id == id).unwrap();
            let s = t0.elapsed().as_secs_f64();
            samples.push((s, t.downloaded));
            if matches!(t.state.status, Status::Completed | Status::Error) {
                break format!("{:?}", t.state.status).to_lowercase();
            }
            if s > 150.0 {
                break "timeout".into();
            }
        };
        let wall = t0.elapsed().as_secs_f64();
        let (u1, k1) = cpu();
        // steady rate between 10 % and 90 % of the file, so setup and finalisation are left out
        let a = samples.iter().find(|(_, b)| *b >= size / 10);
        let z = samples.iter().find(|(_, b)| *b >= size / 10 * 9);
        let steady = match (a, z) {
            (Some(a), Some(z)) if z.0 > a.0 => (z.1 - a.1) as f64 / (z.0 - a.0) / 1e6,
            _ => f64::NAN,
        };
        println!(
            "{:>5} | {:>9} | {:>8.2} | {:>14.0} | {:>17.0} | {:>9.2} | {:>9.2} | {:>11.0}",
            c,
            status,
            wall,
            size as f64 / wall / 1e6,
            steady,
            u1 - u0,
            k1 - k0,
            ((u1 - u0) + (k1 - k0)) / wall * 100.0
        );
        m.shutdown().await;
    }
    println!("Pic mémoire du processus moteur : {:.0} Mio", peak_mib());
    let _ = srv.kill();
    let _ = srv.wait();
    let _ = std::fs::remove_dir_all(base);
}
