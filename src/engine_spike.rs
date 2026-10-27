//! Spike harness and headless server for the shared transcription engine service.

use std::fs;
use std::io::{BufRead, BufReader, Cursor, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use clap::{Args, Subcommand};
use color_eyre::eyre::{Result, WrapErr, bail, eyre};
use hound::{SampleFormat, WavReader, WavSpec, WavWriter};
use serde::{Deserialize, Serialize};

use crate::events::EventLog;
use crate::local_api::{LocalApi, LocalApiOptions};
use crate::parakeet::{Parakeet, prioritize_inference_thread};
use crate::transcription::{Transcriber, WarmTranscriber};
use crate::transcription_engine::TranscriptionEngine;
use crate::transcription_models::{TranscriptionModelId, TranscriptionSelection};

#[derive(Args)]
pub struct EngineArgs {
    #[command(subcommand)]
    pub action: Option<EngineAction>,
    /// Run as an embedded child whose endpoint is printed as JSON on stdout.
    #[arg(long)]
    pub embedded: bool,
    /// Skip raising the engine worker thread to user-interactive QoS.
    #[arg(long)]
    pub no_thread_qos: bool,
}

#[derive(Subcommand)]
pub enum EngineAction {
    /// Run the deterministic spike benchmark suite (isolated from user data).
    Bench,
    /// Test SMAppService.agent registration from a test bundle and unregister.
    TestSmappservice {
        #[arg(long, default_value = "anomaly.hex.engine-spike.plist")]
        plist: String,
    },
}

#[derive(Clone, Copy, Debug, Serialize)]
struct MemorySnapshot {
    rss_mib: f64,
    phys_footprint_mib: f64,
}

#[derive(Clone, Debug, Serialize)]
struct TimingStats {
    runs: Vec<f64>,
    min_ms: f64,
    median_ms: f64,
    p95_ms: f64,
    max_ms: f64,
}

impl TimingStats {
    fn from_ms(mut runs: Vec<f64>) -> Self {
        runs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let n = runs.len();
        let min_ms = runs.first().copied().unwrap_or(0.0);
        let max_ms = runs.last().copied().unwrap_or(0.0);
        let median_ms = if n == 0 {
            0.0
        } else if n % 2 == 1 {
            runs[n / 2]
        } else {
            (runs[n / 2 - 1] + runs[n / 2]) / 2.0
        };
        let p95_idx = ((n as f64 * 0.95).ceil() as usize)
            .saturating_sub(1)
            .min(n.saturating_sub(1));
        let p95_ms = runs.get(p95_idx).copied().unwrap_or(0.0);
        Self {
            runs,
            min_ms,
            median_ms,
            p95_ms,
            max_ms,
        }
    }
}

pub fn run(args: EngineArgs, shutdown: &'static AtomicBool) -> Result<()> {
    match args.action {
        Some(EngineAction::Bench) => run_bench(),
        Some(EngineAction::TestSmappservice { plist }) => run_smappservice_test(&plist),
        None => run_server(args.embedded, !args.no_thread_qos, shutdown),
    }
}

fn install_signal_handlers(shutdown: &'static AtomicBool) {
    static SIGNAL_SHUTDOWN: AtomicBool = AtomicBool::new(false);
    extern "C" fn on_signal(_sig: libc::c_int) {
        SIGNAL_SHUTDOWN.store(true, Ordering::Release);
    }
    unsafe {
        libc::signal(libc::SIGTERM, on_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGINT, on_signal as *const () as libc::sighandler_t);
    }
    thread::Builder::new()
        .name("engine-signal-watch".into())
        .spawn(move || {
            while !shutdown.load(Ordering::Acquire) {
                if SIGNAL_SHUTDOWN.load(Ordering::Acquire) {
                    shutdown.store(true, Ordering::Release);
                    break;
                }
                thread::sleep(Duration::from_millis(50));
            }
        })
        .ok();
}

fn run_server(embedded: bool, raise_thread_qos: bool, shutdown: &'static AtomicBool) -> Result<()> {
    install_signal_handlers(shutdown);
    let support = crate::app_paths::support_dir()?;
    fs::create_dir_all(support.join("logs"))?;
    let event_path = if embedded {
        support
            .join("logs")
            .join(format!("engine-embedded-{}.ndjson", std::process::id()))
    } else {
        support.join("logs").join("engine.ndjson")
    };
    let events = EventLog::create(&event_path)?;

    let selection = TranscriptionSelection {
        model: TranscriptionModelId::ParakeetUnifiedEnglish,
        language: "en".into(),
        recognition_hints: String::new(),
    };

    let t0 = Instant::now();
    let engine = Arc::new(TranscriptionEngine::start(WarmTranscriber::default())?);
    let (loaded_tx, loaded_rx) = mpsc::sync_channel(1);
    let load_selection = selection.clone();
    engine.run(move |model| {
        if raise_thread_qos {
            prioritize_inference_thread();
        }
        let result = Parakeet::load_selection_timed(&load_selection).map(
            |(parakeet, load_ms, prewarm_ms, probe_ms)| {
                *model = WarmTranscriber::serving(Transcriber::Gguf(Box::new(parakeet)));
                (load_ms, prewarm_ms, probe_ms)
            },
        );
        let _ = loaded_tx.send(result);
    })?;
    let (load_ms, prewarm_ms, probe_ms) = loaded_rx
        .recv()
        .wrap_err("engine worker exited during model load")??;
    engine.share();
    tracing::info!(
        total_ms = t0.elapsed().as_millis(),
        load_ms,
        prewarm_ms,
        probe_ms,
        "shared engine ready"
    );

    let local_api = if embedded {
        let api = LocalApi::start_embedded(events)?;
        thread::Builder::new()
            .name("embedded-engine-lease".into())
            .spawn(move || {
                let mut stdin = std::io::stdin().lock();
                let mut buffer = [0_u8; 1];
                loop {
                    match stdin.read(&mut buffer) {
                        Ok(0) => {
                            shutdown.store(true, Ordering::Release);
                            thread::sleep(Duration::from_secs(5));
                            unsafe { libc::_exit(0) };
                        }
                        Ok(_) => {}
                        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                        Err(_) => {
                            shutdown.store(true, Ordering::Release);
                            break;
                        }
                    }
                }
            })?;
        let mut stdout = std::io::stdout().lock();
        serde_json::to_writer(&mut stdout, &api.embedded_endpoint())?;
        stdout.write_all(b"\n")?;
        stdout.flush()?;
        drop(stdout);
        api
    } else {
        LocalApi::start(events, LocalApiOptions::default())?
    };

    while !shutdown.load(Ordering::Acquire) {
        thread::sleep(Duration::from_millis(50));
    }
    drop(local_api);
    drop(engine);
    if embedded {
        let _ = fs::remove_file(&event_path);
    }
    unsafe { libc::_exit(0) }
}

/// Queries the current process's RSS (`resident_size`) and `phys_footprint` from Mach.
fn current_memory() -> MemorySnapshot {
    #[repr(C)]
    struct MachTaskBasicInfo {
        virtual_size: u64,
        resident_size: u64,
        resident_size_max: u64,
        user_time: [u32; 2],
        system_time: [u32; 2],
        policy: i32,
        suspend_count: i32,
    }
    #[repr(C)]
    struct TaskVmInfo {
        virtual_size: u64,
        region_count: i32,
        page_size: i32,
        resident_size: u64,
        resident_size_peak: u64,
        device: u64,
        device_peak: u64,
        internal: u64,
        internal_peak: u64,
        external: u64,
        external_peak: u64,
        reusable: u64,
        reusable_peak: u64,
        purgeable_volatile_pmap: u64,
        purgeable_volatile_resident: u64,
        purgeable_volatile_virtual: u64,
        compressed: u64,
        compressed_peak: u64,
        compressed_lifetime: u64,
        phys_footprint: u64,
        _padding: [u64; 32],
    }
    const MACH_TASK_BASIC_INFO: u32 = 20;
    const TASK_VM_INFO: u32 = 22;
    unsafe extern "C" {
        fn mach_task_self() -> u32;
        fn task_info(
            target_task: u32,
            flavor: u32,
            task_info_out: *mut libc::c_void,
            task_info_out_cnt: *mut u32,
        ) -> i32;
    }
    let mut basic: MachTaskBasicInfo = unsafe { std::mem::zeroed() };
    let mut basic_cnt = (size_of::<MachTaskBasicInfo>() / size_of::<u32>()) as u32;
    let basic_rc = unsafe {
        task_info(
            mach_task_self(),
            MACH_TASK_BASIC_INFO,
            (&raw mut basic).cast(),
            &raw mut basic_cnt,
        )
    };
    let mut vm: TaskVmInfo = unsafe { std::mem::zeroed() };
    let mut vm_cnt = (size_of::<TaskVmInfo>() / size_of::<u32>()) as u32;
    let vm_rc = unsafe {
        task_info(
            mach_task_self(),
            TASK_VM_INFO,
            (&raw mut vm).cast(),
            &raw mut vm_cnt,
        )
    };
    let rss_mib = if basic_rc == 0 {
        basic.resident_size as f64 / (1024.0 * 1024.0)
    } else {
        0.0
    };
    let phys_footprint_mib = if vm_rc == 0 {
        vm.phys_footprint as f64 / (1024.0 * 1024.0)
    } else {
        0.0
    };
    MemorySnapshot {
        rss_mib: (rss_mib * 10.0).round() / 10.0,
        phys_footprint_mib: (phys_footprint_mib * 10.0).round() / 10.0,
    }
}

fn setup_isolated_support_dir() -> Result<PathBuf> {
    let base = PathBuf::from("/private/var/folders/4t/_jnrck017r5f_jttbc19b7s40000gn/T/opencode");
    let support = base.join(format!("hex-engine-spike-{}", std::process::id()));
    let _ = fs::remove_dir_all(&support);
    fs::create_dir_all(support.join("models"))?;
    fs::create_dir_all(support.join("logs"))?;

    let home = dirs::home_dir().ok_or_else(|| eyre!("home_dir unavailable"))?;
    let candidates = [
        home.join("Library/Application Support/hex-test/models"),
        home.join("Library/Application Support/hex/models"),
    ];
    let src_models = candidates
        .iter()
        .find(|dir| dir.join("parakeet-unified-en-0.6b-Q8_0.gguf").is_file())
        .ok_or_else(|| {
            eyre!("parakeet-unified-en-0.6b-Q8_0.gguf not found in Application Support")
        })?;

    for name in [
        "parakeet-unified-en-0.6b-Q8_0.gguf",
        "parakeet-unified-en-0.6b-Q8_0.gguf.verified.json",
    ] {
        std::os::unix::fs::symlink(src_models.join(name), support.join("models").join(name))?;
    }
    unsafe {
        std::env::set_var("HEX_APPLICATION_SUPPORT_DIR", &support);
    }
    Ok(support)
}

fn read_wav_f32_and_bytes(path: &Path) -> Result<(Vec<f32>, Vec<u8>, u64)> {
    let bytes = fs::read(path)?;
    let mut reader = WavReader::new(Cursor::new(&bytes))?;
    let spec = reader.spec();
    if spec.sample_rate != 16_000 || spec.channels != 1 {
        bail!("expected 16 kHz mono WAV fixture: {}", path.display());
    }
    let samples: Vec<f32> = match spec.sample_format {
        SampleFormat::Int => reader
            .samples::<i16>()
            .map(|s| s.map(|v| f32::from(v) / 32_768.0))
            .collect::<std::result::Result<Vec<_>, _>>()?,
        SampleFormat::Float => reader
            .samples::<f32>()
            .collect::<std::result::Result<Vec<_>, _>>()?,
    };
    let duration_ms = samples.len() as u64 * 1000 / 16_000;
    Ok((samples, bytes, duration_ms))
}

fn build_concat_wav(paths: &[PathBuf]) -> Result<(Vec<f32>, Vec<u8>, u64)> {
    let mut all_samples = Vec::new();
    for path in paths {
        let (samples, _, _) = read_wav_f32_and_bytes(path)?;
        all_samples.extend_from_slice(&samples);
        // 200 ms pause between sentences
        all_samples.extend(std::iter::repeat_n(0.0_f32, 3_200));
    }
    let duration_ms = all_samples.len() as u64 * 1000 / 16_000;
    let mut cursor = Cursor::new(Vec::new());
    {
        let mut writer = WavWriter::new(
            &mut cursor,
            WavSpec {
                channels: 1,
                sample_rate: 16_000,
                bits_per_sample: 16,
                sample_format: SampleFormat::Int,
            },
        )?;
        for &s in &all_samples {
            let clamped = (s.clamp(-1.0, 1.0) * 32_767.0) as i16;
            writer.write_sample(clamped)?;
        }
        writer.finalize()?;
    }
    Ok((all_samples, cursor.into_inner(), duration_ms))
}

struct ChildEngine {
    child: Child,
    port: u16,
    token: String,
    startup_ms: f64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReadyLine {
    url: String,
    token: String,
}

impl ChildEngine {
    fn spawn(support_dir: &Path, background_policy: bool) -> Result<Self> {
        let exe = std::env::current_exe()?;
        let t0 = Instant::now();
        let mut cmd = if background_policy {
            let mut c = Command::new("/usr/sbin/taskpolicy");
            c.args(["-b", exe.to_str().unwrap(), "engine", "--embedded"]);
            c
        } else {
            let mut c = Command::new(&exe);
            c.args(["engine", "--embedded"]);
            c
        };
        cmd.env("HEX_APPLICATION_SUPPORT_DIR", support_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = cmd.spawn().wrap_err("could not spawn child engine")?;
        let stdout = child.stdout.take().unwrap();
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        reader.read_line(&mut line)?;
        let startup_ms = t0.elapsed().as_secs_f64() * 1000.0;
        let ready: ReadyLine = serde_json::from_str(line.trim())
            .wrap_err_with(|| format!("bad ready line: {line}"))?;
        let port: u16 = ready
            .url
            .rsplit(':')
            .next()
            .ok_or_else(|| eyre!("missing port"))?
            .parse()?;
        Ok(Self {
            child,
            port,
            token: ready.token,
            startup_ms,
        })
    }

    fn request(
        &self,
        method: &str,
        path: &str,
        extra_headers: &str,
        body: &[u8],
    ) -> Result<(u16, String)> {
        let mut stream = TcpStream::connect(("127.0.0.1", self.port))?;
        stream.set_nodelay(true)?;
        stream.set_read_timeout(Some(Duration::from_secs(30)))?;
        let header = format!(
            "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {}\r\n{extra_headers}Content-Length: {}\r\n\r\n",
            self.token,
            body.len()
        );
        stream.write_all(header.as_bytes())?;
        if !body.is_empty() {
            stream.write_all(body)?;
        }
        stream.flush()?;
        let mut response = Vec::new();
        stream.read_to_end(&mut response)?;
        let text = String::from_utf8_lossy(&response);
        let status: u16 = text
            .get(9..12)
            .ok_or_else(|| eyre!("malformed HTTP status"))?
            .parse()?;
        let resp_body = text
            .split_once("\r\n\r\n")
            .map(|(_, b)| b.to_string())
            .unwrap_or_default();
        Ok((status, resp_body))
    }

    fn transcribe_wav(&self, wav_bytes: &[u8]) -> Result<String> {
        let (status, body) = self.request(
            "POST",
            "/transcriptions?model=parakeet_unified_en&language=en",
            "Content-Type: audio/wav\r\n",
            wav_bytes,
        )?;
        if status != 200 {
            bail!("POST /transcriptions failed ({status}): {body}");
        }
        #[derive(Deserialize)]
        struct Resp {
            transcript: String,
        }
        let parsed: Resp = serde_json::from_str(&body)?;
        Ok(parsed.transcript)
    }
}

impl Drop for ChildEngine {
    fn drop(&mut self) {
        let _ = self.child.stdin.take();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[derive(Serialize)]
struct StreamIpcMetrics {
    create_ms: f64,
    audio_push_us_median: f64,
    audio_push_us_p95: f64,
    partial_count: usize,
    partial_latency_ms: TimingStats,
    finish_ms: f64,
    final_text: String,
}

fn measure_http_stream(
    server: &ChildEngine,
    samples: &[f32],
    chunk_interval: Duration,
) -> Result<StreamIpcMetrics> {
    let t_create = Instant::now();
    let create_body = br#"{"sampleFormat":"f32le","sampleRate":16000,"model":"parakeet_unified_en","language":"en"}"#;
    let (status, body) = server.request(
        "POST",
        "/streams",
        "Content-Type: application/json\r\n",
        create_body,
    )?;
    let create_ms = t_create.elapsed().as_secs_f64() * 1000.0;
    if status != 201 {
        bail!("POST /streams failed ({status}): {body}");
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Created {
        id: u64,
        owner_token: String,
    }
    let created: Created = serde_json::from_str(&body)?;

    // Open SSE connection and record arrival time of each partial event.
    let mut sse_conn = TcpStream::connect(("127.0.0.1", server.port))?;
    sse_conn.set_nodelay(true)?;
    let sse_req = format!(
        "GET /streams/{}/events HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {}\r\nX-Hex-Stream-Token: {}\r\n\r\n",
        created.id, server.token, created.owner_token
    );
    sse_conn.write_all(sse_req.as_bytes())?;
    sse_conn.flush()?;

    let last_push_time = Arc::new(Mutex::new(Instant::now()));
    let partial_latencies = Arc::new(Mutex::new(Vec::<f64>::new()));
    let (ready_tx, ready_rx) = mpsc::sync_channel(1);
    let last_push_reader = last_push_time.clone();
    let partials_reader = partial_latencies.clone();

    let sse_thread = thread::spawn(move || {
        let mut reader = BufReader::new(sse_conn);
        let mut line = String::new();
        while reader.read_line(&mut line).unwrap_or(0) > 0 {
            let now = Instant::now();
            if let Some(json_str) = line.strip_prefix("data: ") {
                let json_str = json_str.trim();
                if json_str.contains("\"type\":\"ready\"") {
                    let _ = ready_tx.send(());
                } else if json_str.contains("\"type\":\"partial\"") {
                    let pushed = *last_push_reader.lock().unwrap();
                    let lat_ms = now.saturating_duration_since(pushed).as_secs_f64() * 1000.0;
                    partials_reader.lock().unwrap().push(lat_ms);
                } else if json_str.contains("\"type\":\"final\"")
                    || json_str.contains("\"type\":\"error\"")
                    || json_str.contains("\"type\":\"cancelled\"")
                {
                    break;
                }
            }
            line.clear();
        }
    });

    ready_rx.recv_timeout(Duration::from_secs(5))?;

    let extra = format!(
        "X-Hex-Stream-Token: {}\r\nContent-Type: application/octet-stream\r\n",
        created.owner_token
    );
    let audio_path = format!("/streams/{}/audio", created.id);
    let mut push_us = Vec::new();

    // Push 100 ms (1,600 samples = 6,400 bytes) chunks
    for chunk in samples.chunks(1_600) {
        let mut bytes = Vec::with_capacity(chunk.len() * 4);
        for &s in chunk {
            bytes.extend_from_slice(&s.to_le_bytes());
        }
        *last_push_time.lock().unwrap() = Instant::now();
        let t_push = Instant::now();
        let (st, b) = server.request("POST", &audio_path, &extra, &bytes)?;
        let elapsed_us = t_push.elapsed().as_secs_f64() * 1_000_000.0;
        if st != 204 {
            bail!("POST {audio_path} failed ({st}): {b}");
        }
        push_us.push(elapsed_us);
        if !chunk_interval.is_zero() {
            thread::sleep(chunk_interval);
        }
    }

    let finish_extra = format!("X-Hex-Stream-Token: {}\r\n", created.owner_token);
    let finish_path = format!("/streams/{}/finish", created.id);
    let t_finish = Instant::now();
    let (st, fin_body) = server.request("POST", &finish_path, &finish_extra, b"")?;
    let finish_ms = t_finish.elapsed().as_secs_f64() * 1000.0;
    if st != 200 {
        bail!("POST {finish_path} failed ({st}): {fin_body}");
    }
    let _ = sse_thread.join();

    #[derive(Deserialize)]
    struct FinalResp {
        text: String,
    }
    let fin: FinalResp = serde_json::from_str(&fin_body)?;

    push_us.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let push_med = push_us[push_us.len() / 2];
    let push_p95 = push_us[(push_us.len() * 95 / 100).min(push_us.len() - 1)];
    let partials = partial_latencies.lock().unwrap().clone();

    Ok(StreamIpcMetrics {
        create_ms: (create_ms * 100.0).round() / 100.0,
        audio_push_us_median: (push_med * 10.0).round() / 10.0,
        audio_push_us_p95: (push_p95 * 10.0).round() / 10.0,
        partial_count: partials.len(),
        partial_latency_ms: TimingStats::from_ms(partials),
        finish_ms: (finish_ms * 100.0).round() / 100.0,
        final_text: fin.text,
    })
}

fn run_bench() -> Result<()> {
    let support = setup_isolated_support_dir()?;
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let short_wav_path = root.join("tests/fixtures/vocabulary/audio/term-hex-3.wav");
    let concat_paths = [
        root.join("tests/fixtures/vocabulary/audio/term-hex-1.wav"),
        root.join("tests/fixtures/vocabulary/audio/term-hex-2.wav"),
        root.join("tests/fixtures/vocabulary/audio/term-hex-3.wav"),
        root.join("tests/fixtures/vocabulary/audio/term-abattoir-1.wav"),
        root.join("tests/fixtures/vocabulary/audio/term-opencode-1.wav"),
    ];
    let (short_samples, short_wav_bytes, short_duration_ms) =
        read_wav_f32_and_bytes(&short_wav_path)?;
    let (long_samples, long_wav_bytes, long_duration_ms) = build_concat_wav(&concat_paths)?;

    let selection = TranscriptionSelection {
        model: TranscriptionModelId::ParakeetUnifiedEnglish,
        language: "en".into(),
        recognition_hints: String::new(),
    };

    // 1. In-process memory & cold load / unload / warm reload
    let mem_before_load = current_memory();
    let engine = Arc::new(TranscriptionEngine::start(WarmTranscriber::default())?);

    let load_once = |engine: &TranscriptionEngine,
                     sel: &TranscriptionSelection|
     -> Result<(u128, u128, u128, u128)> {
        let (tx, rx) = mpsc::sync_channel(1);
        let sel = sel.clone();
        let t_total = Instant::now();
        engine.run(move |model| {
            prioritize_inference_thread();
            let res = Parakeet::load_selection_timed(&sel).map(
                |(parakeet, load_ms, prewarm_ms, probe_ms)| {
                    *model = WarmTranscriber::serving(Transcriber::Gguf(Box::new(parakeet)));
                    (load_ms, prewarm_ms, probe_ms)
                },
            );
            let _ = tx.send(res);
        })?;
        let (load_ms, prewarm_ms, probe_ms) = rx.recv()??;
        let total_ms = t_total.elapsed().as_millis();
        Ok((total_ms, load_ms, prewarm_ms, probe_ms))
    };

    let (cold_total_ms, cold_load_ms, cold_prewarm_ms, cold_probe_ms) =
        load_once(&engine, &selection)?;
    let mem_after_cold_load = current_memory();

    // 2. In-process warm transcription (1 warmup + 7 runs)
    let transcribe_inproc = |samples: &[f32]| -> Result<(String, f64)> {
        let clip = samples.to_vec();
        let sel = selection.clone();
        let t0 = Instant::now();
        let pending = engine.foreground(
            clip,
            Box::new(move |model, samples| {
                let transcriber = model.activate(&sel).map_err(|e| e.to_string())?;
                transcriber.transcribe(samples).map_err(|e| e.to_string())
            }),
        )?;
        let res = pending.wait().map_err(|e| eyre!(e))?;
        Ok((res.text, t0.elapsed().as_secs_f64() * 1000.0))
    };

    let (inproc_short_text, _) = transcribe_inproc(&short_samples)?;
    let mut inproc_short_runs = Vec::new();
    for _ in 0..7 {
        let (txt, ms) = transcribe_inproc(&short_samples)?;
        assert_eq!(txt, inproc_short_text);
        inproc_short_runs.push((ms * 100.0).round() / 100.0);
    }

    let (inproc_long_text, _) = transcribe_inproc(&long_samples)?;
    let mut inproc_long_runs = Vec::new();
    for _ in 0..7 {
        let (txt, ms) = transcribe_inproc(&long_samples)?;
        assert_eq!(txt, inproc_long_text);
        inproc_long_runs.push((ms * 100.0).round() / 100.0);
    }
    let mem_after_inference = current_memory();

    // 3. In-process LiveStream (paced 100 ms chunks every 25 ms, then finish)
    let stream_inproc = |samples: &[f32], interval: Duration| -> Result<(String, f64)> {
        let mut live = engine
            .begin()
            .ok_or_else(|| eyre!("engine did not open LiveStream"))?;
        for chunk in samples.chunks(1_600) {
            live.feed(chunk);
            if !interval.is_zero() {
                thread::sleep(interval);
            }
        }
        let sel = selection.clone();
        let t_fin = Instant::now();
        let pending = engine.finish_with(
            Some(live),
            samples.to_vec(),
            Box::new(move |model, clip| {
                let transcriber = model.activate(&sel).map_err(|e| e.to_string())?;
                transcriber.transcribe(clip).map_err(|e| e.to_string())
            }),
        )?;
        let res = pending.wait().map_err(|e| eyre!(e))?;
        Ok((res.text, t_fin.elapsed().as_secs_f64() * 1000.0))
    };

    let _ = stream_inproc(&long_samples, Duration::from_millis(25))?;
    let mut inproc_stream_finish_runs = Vec::new();
    for _ in 0..5 {
        let (_, fin_ms) = stream_inproc(&long_samples, Duration::from_millis(25))?;
        inproc_stream_finish_runs.push((fin_ms * 100.0).round() / 100.0);
    }

    // 4. Idle model unload & warm reload in-process
    let (unload_tx, unload_rx) = mpsc::sync_channel(1);
    let t_unload = Instant::now();
    engine.run(move |model| {
        *model = WarmTranscriber::default();
        let _ = unload_tx.send(());
    })?;
    unload_rx.recv()?;
    let unload_ms = t_unload.elapsed().as_secs_f64() * 1000.0;
    let mem_after_unload_raw = current_memory();
    unsafe extern "C" {
        fn malloc_zone_pressure_relief(zone: *mut libc::c_void, goal: usize) -> usize;
    }
    unsafe {
        malloc_zone_pressure_relief(std::ptr::null_mut(), 0);
    }
    let mem_after_unload = current_memory();

    let (warm_total_ms, warm_load_ms, warm_prewarm_ms, warm_probe_ms) =
        load_once(&engine, &selection)?;
    let mem_after_reload = current_memory();

    // Free in-process engine before spawning child engines so we don't hold two copies in RAM
    drop(engine);

    // 5. Out-of-process child engine over loopback HTTP (normal Interactive QoS)
    let child_normal = ChildEngine::spawn(&support, false)?;
    let normal_startup_ms = child_normal.startup_ms;

    // Measure GET /health round-trip latency
    let _ = child_normal.request("GET", "/health", "", b"")?;
    let mut health_us = Vec::new();
    for _ in 0..30 {
        let t0 = Instant::now();
        let (st, _) = child_normal.request("GET", "/health", "", b"")?;
        assert_eq!(st, 200);
        health_us.push((t0.elapsed().as_secs_f64() * 1_000_000.0 * 10.0).round() / 10.0);
    }
    health_us.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let health_median_us = health_us[health_us.len() / 2];
    let health_p95_us = health_us[(health_us.len() * 95 / 100).min(health_us.len() - 1)];

    // Measure POST /transcriptions over loopback HTTP (1 warmup + 7 runs)
    let ipc_short_text = child_normal.transcribe_wav(&short_wav_bytes)?;
    assert_eq!(ipc_short_text, inproc_short_text);
    let mut ipc_short_runs = Vec::new();
    for _ in 0..7 {
        let t0 = Instant::now();
        let txt = child_normal.transcribe_wav(&short_wav_bytes)?;
        let ms = t0.elapsed().as_secs_f64() * 1000.0;
        assert_eq!(txt, inproc_short_text);
        ipc_short_runs.push((ms * 100.0).round() / 100.0);
    }

    let ipc_long_text = child_normal.transcribe_wav(&long_wav_bytes)?;
    assert_eq!(ipc_long_text, inproc_long_text);
    let mut ipc_long_runs = Vec::new();
    for _ in 0..7 {
        let t0 = Instant::now();
        let txt = child_normal.transcribe_wav(&long_wav_bytes)?;
        let ms = t0.elapsed().as_secs_f64() * 1000.0;
        assert_eq!(txt, inproc_long_text);
        ipc_long_runs.push((ms * 100.0).round() / 100.0);
    }

    // Measure /streams over loopback HTTP (paced 100 ms chunks every 25 ms)
    let _ = measure_http_stream(&child_normal, &long_samples, Duration::from_millis(25))?;
    let mut ipc_stream_finish_runs = Vec::new();
    let mut ipc_stream_last = None;
    for _ in 0..5 {
        let m = measure_http_stream(&child_normal, &long_samples, Duration::from_millis(25))?;
        ipc_stream_finish_runs.push(m.finish_ms);
        ipc_stream_last = Some(m);
    }
    let ipc_stream_sample = ipc_stream_last.unwrap();

    // Measure normal child under 12-thread CPU contention
    let stop_spin = Arc::new(AtomicBool::new(false));
    let spinners: Vec<_> = (0..12)
        .map(|_| {
            let stop = stop_spin.clone();
            thread::spawn(move || {
                let mut x = 1_u64;
                while !stop.load(Ordering::Relaxed) {
                    x = x.wrapping_mul(6364136223846793005).wrapping_add(1);
                    std::hint::black_box(x);
                }
            })
        })
        .collect();
    let _ = child_normal.transcribe_wav(&long_wav_bytes)?;
    let mut normal_contended_long_runs = Vec::new();
    for _ in 0..7 {
        let t0 = Instant::now();
        let _ = child_normal.transcribe_wav(&long_wav_bytes)?;
        normal_contended_long_runs
            .push((t0.elapsed().as_secs_f64() * 1000.0 * 100.0).round() / 100.0);
    }
    stop_spin.store(true, Ordering::Relaxed);
    for s in spinners {
        let _ = s.join();
    }
    drop(child_normal);

    // 6. Out-of-process child engine under Darwin Background policy (`taskpolicy -b`, simulating default launchd agent QoS)
    let child_bg = ChildEngine::spawn(&support, true)?;
    let _ = child_bg.transcribe_wav(&short_wav_bytes)?;
    let mut bg_quiet_short_runs = Vec::new();
    for _ in 0..7 {
        let t0 = Instant::now();
        let _ = child_bg.transcribe_wav(&short_wav_bytes)?;
        bg_quiet_short_runs.push((t0.elapsed().as_secs_f64() * 1000.0 * 100.0).round() / 100.0);
    }
    let _ = child_bg.transcribe_wav(&long_wav_bytes)?;
    let mut bg_quiet_long_runs = Vec::new();
    for _ in 0..7 {
        let t0 = Instant::now();
        let _ = child_bg.transcribe_wav(&long_wav_bytes)?;
        bg_quiet_long_runs.push((t0.elapsed().as_secs_f64() * 1000.0 * 100.0).round() / 100.0);
    }

    // Background child under 12-thread CPU contention
    let stop_spin2 = Arc::new(AtomicBool::new(false));
    let spinners2: Vec<_> = (0..12)
        .map(|_| {
            let stop = stop_spin2.clone();
            thread::spawn(move || {
                let mut x = 1_u64;
                while !stop.load(Ordering::Relaxed) {
                    x = x.wrapping_mul(6364136223846793005).wrapping_add(1);
                    std::hint::black_box(x);
                }
            })
        })
        .collect();
    let _ = child_bg.transcribe_wav(&long_wav_bytes)?;
    let mut bg_contended_long_runs = Vec::new();
    for _ in 0..7 {
        let t0 = Instant::now();
        let _ = child_bg.transcribe_wav(&long_wav_bytes)?;
        bg_contended_long_runs.push((t0.elapsed().as_secs_f64() * 1000.0 * 100.0).round() / 100.0);
    }
    stop_spin2.store(true, Ordering::Relaxed);
    for s in spinners2 {
        let _ = s.join();
    }
    let bg_startup_ms = child_bg.startup_ms;
    drop(child_bg);

    let _ = fs::remove_dir_all(&support);

    let report = serde_json::json!({
        "fixtures": {
            "short_duration_ms": short_duration_ms,
            "short_wav_bytes": short_wav_bytes.len(),
            "short_transcript": inproc_short_text,
            "long_duration_ms": long_duration_ms,
            "long_wav_bytes": long_wav_bytes.len(),
            "long_transcript": inproc_long_text,
        },
        "memory": {
            "before_load": mem_before_load,
            "after_cold_load": mem_after_cold_load,
            "after_inference": mem_after_inference,
            "after_idle_unload_raw": mem_after_unload_raw,
            "after_idle_unload": mem_after_unload,
            "after_warm_reload": mem_after_reload,
            "unload_ms": (unload_ms * 100.0).round() / 100.0,
        },
        "model_load_ms": {
            "cold": {
                "total_ms": cold_total_ms,
                "metal_load_ms": cold_load_ms,
                "offline_prewarm_1500ms": cold_prewarm_ms,
                "stream_prewarm_8000ms": cold_probe_ms,
            },
            "warm_reload_in_process": {
                "total_ms": warm_total_ms,
                "metal_load_ms": warm_load_ms,
                "offline_prewarm_1500ms": warm_prewarm_ms,
                "stream_prewarm_8000ms": warm_probe_ms,
            },
        },
        "ipc_baseline": {
            "health_get_median_us": health_median_us,
            "health_get_p95_us": health_p95_us,
        },
        "transcription_short_2640ms": {
            "in_process_ms": TimingStats::from_ms(inproc_short_runs),
            "loopback_http_ms": TimingStats::from_ms(ipc_short_runs),
        },
        "transcription_long_11920ms": {
            "in_process_ms": TimingStats::from_ms(inproc_long_runs),
            "loopback_http_ms": TimingStats::from_ms(ipc_long_runs),
        },
        "streams_long_11920ms_paced": {
            "in_process_live_stream_finish_ms": TimingStats::from_ms(inproc_stream_finish_runs),
            "loopback_http_stream_finish_ms": TimingStats::from_ms(ipc_stream_finish_runs),
            "sample_run": ipc_stream_sample,
        },
        "qos_comparison": {
            "interactive_child_startup_ms": (normal_startup_ms * 10.0).round() / 10.0,
            "background_taskpolicy_b_startup_ms": (bg_startup_ms * 10.0).round() / 10.0,
            "short_2640ms_background_quiet_ms": TimingStats::from_ms(bg_quiet_short_runs),
            "long_11920ms_background_quiet_ms": TimingStats::from_ms(bg_quiet_long_runs),
            "long_11920ms_interactive_contended_ms": TimingStats::from_ms(normal_contended_long_runs),
            "long_11920ms_background_contended_ms": TimingStats::from_ms(bg_contended_long_runs),
        }
    });

    println!("{}", serde_json::to_string_pretty(&report)?);
    unsafe { libc::_exit(0) }
}

fn run_smappservice_test(plist: &str) -> Result<()> {
    use objc2_foundation::NSString;
    use objc2_service_management::{SMAppService, SMAppServiceStatus};

    let plist_name = NSString::from_str(plist);
    let service = unsafe { SMAppService::agentServiceWithPlistName(&plist_name) };
    let initial_status = unsafe { service.status() };

    let register_result =
        unsafe { service.registerAndReturnError() }.map_err(|err| err.to_string());
    let status_after_register = unsafe { service.status() };

    // Wait up to 8s for launchd to start the agent and write local-api.json in hex-test
    let support = crate::app_paths::support_dir()?;
    let discovery_path = support.join("local-api.json");
    let deadline = Instant::now() + Duration::from_secs(8);
    let mut agent_ready = false;
    while Instant::now() < deadline {
        if discovery_path.is_file() {
            agent_ready = true;
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }

    let mut agent_bench = serde_json::Value::Null;
    if agent_ready {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Disc {
            port: u16,
            token: String,
            pid: u32,
        }
        if let Ok(bytes) = fs::read(&discovery_path)
            && let Ok(disc) = serde_json::from_slice::<Disc>(&bytes)
        {
            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            let short_wav_path = root.join("tests/fixtures/vocabulary/audio/term-hex-3.wav");
            let (_, short_wav_bytes, _) = read_wav_f32_and_bytes(&short_wav_path)?;
            let concat_paths = [
                root.join("tests/fixtures/vocabulary/audio/term-hex-1.wav"),
                root.join("tests/fixtures/vocabulary/audio/term-hex-2.wav"),
                root.join("tests/fixtures/vocabulary/audio/term-hex-3.wav"),
                root.join("tests/fixtures/vocabulary/audio/term-abattoir-1.wav"),
                root.join("tests/fixtures/vocabulary/audio/term-opencode-1.wav"),
            ];
            let (_, long_wav_bytes, _) = build_concat_wav(&concat_paths)?;

            let post_wav = |wav: &[u8]| -> Result<f64> {
                let t0 = Instant::now();
                let mut stream = TcpStream::connect(("127.0.0.1", disc.port))?;
                stream.set_nodelay(true)?;
                stream.set_read_timeout(Some(Duration::from_secs(30)))?;
                let req = format!(
                    "POST /transcriptions?model=parakeet_unified_en&language=en HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {}\r\nContent-Type: audio/wav\r\nContent-Length: {}\r\n\r\n",
                    disc.token,
                    wav.len()
                );
                stream.write_all(req.as_bytes())?;
                stream.write_all(wav)?;
                stream.flush()?;
                let mut resp = Vec::new();
                stream.read_to_end(&mut resp)?;
                Ok((t0.elapsed().as_secs_f64() * 1000.0 * 100.0).round() / 100.0)
            };

            let _ = post_wav(&short_wav_bytes);
            let mut short_runs = Vec::new();
            for _ in 0..7 {
                if let Ok(ms) = post_wav(&short_wav_bytes) {
                    short_runs.push(ms);
                }
            }
            let _ = post_wav(&long_wav_bytes);
            let mut long_runs = Vec::new();
            for _ in 0..7 {
                if let Ok(ms) = post_wav(&long_wav_bytes) {
                    long_runs.push(ms);
                }
            }
            agent_bench = serde_json::json!({
                "agent_pid": disc.pid,
                "agent_port": disc.port,
                "short_2640ms": TimingStats::from_ms(short_runs),
                "long_11920ms": TimingStats::from_ms(long_runs),
            });
        }
    }

    let unregister_result =
        unsafe { service.unregisterAndReturnError() }.map_err(|err| err.to_string());
    let status_after_unregister = unsafe { service.status() };
    let _ = fs::remove_file(&discovery_path);

    let status_name = |s: SMAppServiceStatus| match s {
        SMAppServiceStatus::NotRegistered => "NotRegistered",
        SMAppServiceStatus::Enabled => "Enabled",
        SMAppServiceStatus::RequiresApproval => "RequiresApproval",
        SMAppServiceStatus::NotFound => "NotFound",
        _ => "Unknown",
    };

    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "initial_status": status_name(initial_status),
            "register_result": register_result,
            "status_after_register": status_name(status_after_register),
            "agent_ready": agent_ready,
            "agent_bench": agent_bench,
            "unregister_result": unregister_result,
            "status_after_unregister": status_name(status_after_unregister),
        }))?
    );
    unsafe { libc::_exit(0) }
}
