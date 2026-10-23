use serde_json::{Value, json};
use std::{env, error::Error, fs, path::Path, time::Instant};
use transcribe_cpp::{
    Backend, Model, ModelOptions, RunOptions, SessionOptions, StreamOptions, TimestampKind,
};

fn main() -> Result<(), Box<dyn Error>> {
    let args = env::args().collect::<Vec<_>>();
    if args.len() != 5 {
        return Err("usage: parakeet-boost-eval MODEL MANIFEST offline|stream RUNS".into());
    }
    let streaming = match args[3].as_str() {
        "stream" => true,
        "offline" => false,
        _ => return Err("unknown mode".into()),
    };
    let runs: usize = args[4].parse()?;
    if runs == 0 {
        return Err("runs must be positive".into());
    }
    let manifest_path = Path::new(&args[2]);
    let manifest: Value = serde_json::from_slice(&fs::read(manifest_path)?)?;
    let mut clips = Vec::new();
    for clip in manifest["clips"].as_array().ok_or("clips missing")? {
        let path = manifest_path
            .parent()
            .unwrap()
            .join(clip["audio"].as_str().ok_or("audio missing")?);
        let mut reader = hound::WavReader::open(path)?;
        let spec = reader.spec();
        if spec.channels != 1 || spec.sample_rate != 16000 {
            return Err("runner needs mono 16 kHz WAVs".into());
        }
        let mut samples: Vec<f32> = if spec.sample_format == hound::SampleFormat::Float {
            reader.samples::<f32>().collect::<Result<_, _>>()?
        } else {
            let scale = 2_f32.powi(i32::from(spec.bits_per_sample) - 1);
            reader
                .samples::<i32>()
                .map(|x| x.map(|s| s as f32 / scale))
                .collect::<Result<_, _>>()?
        };
        if samples.is_empty() || samples.iter().any(|s| !s.is_finite()) {
            return Err("invalid audio".into());
        }
        let audio_ms = samples.len() as f64 / 16.0;
        // Hex's minimum whole-clip padding; streams consume exactly the clip.
        if !streaming {
            samples.resize(samples.len().max(24000), 0.0);
        }
        clips.push((
            clip["id"].as_str().ok_or("id missing")?.to_owned(),
            samples,
            audio_ms,
        ));
    }
    transcribe_cpp::disable_logging();
    let model = Model::load_with(
        &args[1],
        &ModelOptions {
            backend: Backend::Metal,
            device: None,
        },
    )?;
    if model.device()?.kind != "metal" {
        return Err("strict Metal required".into());
    }
    let options = RunOptions {
        timestamps: TimestampKind::Word,
        ..Default::default()
    };
    for term in manifest["vocabulary"]
        .as_array()
        .ok_or("vocabulary missing")?
    {
        let term = term.as_str().ok_or("invalid vocabulary term")?;
        let lower = term.to_ascii_lowercase();
        let mut capital = lower.clone();
        if let Some(first) = capital.get_mut(..1) {
            first.make_ascii_uppercase();
        }
        let mut variants = vec![term.to_owned(), lower, capital];
        variants.sort();
        variants.dedup();
        for variant in variants {
            println!(
                "{}",
                json!({"type":"tokenization", "term":variant, "ids":model.tokenize(&variant)?})
            );
        }
    }
    if env::var_os("HEX_BOOST_TOKENIZE_ONLY").is_some() {
        return Ok(());
    }
    // One whole-corpus warmup followed by repeated measurements. A fresh native
    // session for every clip matches OfflineGgufSession's lifetime.
    for run in 0..=runs {
        for (id, samples, audio_ms) in &clips {
            let start = Instant::now();
            let mut session = model.session_with(&SessionOptions {
                n_threads: 1,
                ..Default::default()
            })?;
            let (text, finish_ms) = if streaming {
                let mut stream = session.stream(&options, &StreamOptions::default())?;
                for block in samples.chunks(320) {
                    stream.feed(block)?;
                }
                let finish = Instant::now();
                stream.finalize()?;
                (stream.text().full, finish.elapsed().as_secs_f64() * 1000.0)
            } else {
                (session.run(samples, &options)?.text, 0.0)
            };
            drop(session);
            if run > 0 {
                println!(
                    "{}",
                    json!({"type":"clip", "id":id, "run":run,
                    "mode":args[3], "transcript":text, "audio_ms":audio_ms,
                    "inference_ms":start.elapsed().as_secs_f64()*1000.0, "finish_ms":finish_ms})
                );
            }
        }
    }
    Ok(())
}
