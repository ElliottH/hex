use serde_json::{Value, json};
use std::{collections::BTreeMap, env, error::Error, fs, path::Path, time::Instant};
use transcribe_cpp::{
    Backend, Feature, Model, ModelOptions, RunOptions, SessionOptions, StreamOptions, TimestampKind,
};

fn main() -> Result<(), Box<dyn Error>> {
    let args = env::args().collect::<Vec<_>>();
    if args.len() != 3 {
        return Err("usage: hex-model-compare PLAN RUNS".into());
    }
    let runs: usize = args[2].parse()?;
    if runs == 0 {
        return Err("runs must be positive".into());
    }
    let plan: Value = serde_json::from_slice(&fs::read(&args[1])?)?;
    let manifest_path = Path::new(plan["manifest"].as_str().ok_or("manifest missing")?);
    let manifest: Value = serde_json::from_slice(&fs::read(manifest_path)?)?;
    let mut clips = Vec::new();
    for clip in manifest["clips"].as_array().ok_or("clips missing")? {
        let path = manifest_path
            .parent()
            .ok_or("manifest parent missing")?
            .join(clip["audio"].as_str().ok_or("audio missing")?);
        let mut reader = hound::WavReader::open(path)?;
        let spec = reader.spec();
        if spec.channels != 1 || spec.sample_rate != 16000 || spec.bits_per_sample != 16 {
            return Err("runner needs mono 16 kHz PCM16 WAVs".into());
        }
        let samples = reader
            .samples::<i16>()
            .map(|s| s.map(|s| f32::from(s) / 32768.0))
            .collect::<Result<Vec<_>, _>>()?;
        if samples.is_empty() {
            return Err("empty audio".into());
        }
        clips.push((clip["id"].as_str().ok_or("id missing")?.to_owned(), samples));
    }
    let mut models = BTreeMap::new();
    for (name, path) in plan["models"].as_object().ok_or("models missing")? {
        let start = Instant::now();
        let model = Model::load_with(
            path.as_str().ok_or("model path missing")?,
            &ModelOptions {
                backend: Backend::Metal,
                ..Default::default()
            },
        )?;
        if model.device()?.kind != "metal" {
            return Err("strict Metal required".into());
        }
        println!(
            "{}",
            json!({"type":"model", "name":name, "variant":model.variant(),
                "engine":transcribe_cpp::version(), "commit":transcribe_cpp::version_commit(),
                "load_ms":start.elapsed().as_secs_f64()*1000.0,
                "streams":model.capabilities()?.supports_streaming,
                "context_prompt":model.supports(Feature::ContextPrompt)})
        );
        models.insert(name.to_owned(), model);
    }
    let arms = plan["arms"].as_array().ok_or("arms missing")?;
    if arms.is_empty() || clips.is_empty() {
        return Err("empty comparison".into());
    }
    // All models remain resident. Rotate arm order for every clip and pass so
    // time-varying worker contention is not assigned to one sequential arm.
    // Run zero warms every clip/arm; the remaining runs are measured.
    for run in 0..=runs {
        for (index, (id, samples)) in clips.iter().enumerate() {
            let audio_ms = samples.len() as f64 / 16.0;
            let mut padded = samples.clone();
            padded.resize(padded.len().max(24000), 0.0);
            for offset in 0..arms.len() {
                let arm = &arms[(index + run + offset) % arms.len()];
                let name = arm["name"].as_str().ok_or("arm name missing")?;
                let model = models
                    .get(arm["model"].as_str().ok_or("arm model missing")?)
                    .ok_or("unknown arm model")?;
                let streaming = arm["streaming"].as_bool().ok_or("streaming missing")?;
                let prompt = arm["prompt"].as_str().map(str::to_owned);
                if prompt.is_some() && !model.supports(Feature::ContextPrompt) {
                    return Err("model does not support the hint prompt".into());
                }
                let options = RunOptions {
                    timestamps: TimestampKind::None,
                    prompt,
                    ..Default::default()
                };
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
                    (session.run(&padded, &options)?.text, 0.0)
                };
                drop(session);
                if run > 0 {
                    let mut record = json!({"type":"clip", "arm":name, "id":id, "run":run,
                        "transcript":text, "audio_ms":audio_ms,
                        "inference_ms":start.elapsed().as_secs_f64()*1000.0});
                    if streaming {
                        record["finish_ms"] = json!(finish_ms);
                    }
                    println!("{record}");
                }
            }
        }
        eprintln!("completed comparison pass {run}/{runs}");
    }
    Ok(())
}
