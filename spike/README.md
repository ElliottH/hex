# Parakeet phrase-boost spike

This branch carries a patch, not a published engine fork. Hex's production
dependency stays on `transcribe-cpp = 0.2.4`. The isolated runner applies the
patch to a private copy of `transcribe-cpp-sys 0.2.4` through `[patch.crates-io]`.
Using the shipped engine keeps an unrelated 0.4.1 upgrade out of the comparison.

See `docs/research/parakeet-boost-spike.md` for the measured decision.

## Reproduce on an admitted worker

```sh
python3 spike/prepare.py
cargo build --release -j 2 --manifest-path spike/runner/Cargo.toml
c++ -std=c++17 -O2 -Itarget/boost-engine/src -Itarget/boost-engine/include \
  spike/phrase-boost-test.cpp -o target/phrase-boost-test
target/phrase-boost-test

# Build a relative-path combined manifest in perf/transcription-corpus,
# plus vocabulary.txt containing one phrase per line. Keep private clips private.
python3 spike/sweep.py "$CARGO_TARGET_DIR/release/parakeet-boost-eval" \
  /path/to/parakeet-unified-en-0.6b-Q8_0.gguf
python3 spike/meeting-chunks.py "$CARGO_TARGET_DIR/release/parakeet-boost-eval" \
  /path/to/parakeet-unified-en-0.6b-Q8_0.gguf
python3 spike/summarize.py perf/transcription-corpus/results spike/results.json
```

Use HiveMind for builds and inference. It may choose a different admitted worker;
put the pinned model and corpus on that worker before running the sweep. No app
installation, microphone, paste, UI, or speaker device is involved. The runner
requires mono 16 kHz WAVs and strict Metal, one CPU decoder thread, fresh native
sessions, and the default Unified 5.6/1.04/1.04-second stream geometry. Streaming
feeds 20 ms blocks without wall-clock sleeps; it measures compute and finalization,
not elapsed speaking time or shortcut-to-paste latency.

The meeting replay concatenates the committed synthetic sentences with 500 ms
pauses into sentence-aligned chunks up to 20 seconds, then uses fresh offline
sessions. It checks multi-sentence decoder behavior, not native meeting capture,
the scheduler, pause segmentation, echo suppression, or overlap deduplication.
It reuses tuning sentences and is a path smoke, not held-out accuracy evidence.

The model is `handy-computer/parakeet-unified-en-0.6b-gguf` revision
`7e948f21b7bdbac698d3318db9d350f1096f3b6c`, SHA-256
`4b50b6dd862bf6e346929aaf4f5eaacec003bfa3f56462d6c874b41ef2f38795`.

The patch reads `HEX_BOOST_VOCABULARY`, `HEX_BOOST_BONUS` and
`HEX_BOOST_MARGIN` once at model load. Vocabulary is capped at 64 phrases and
128 bytes per phrase. Original, lowercase and initial-capital variants are
encoded. The encoder implements score-ranked SentencePiece BPE and unigram
Viterbi; only ASCII normalization is supported. Non-ASCII input fails rather
than pretending that the GGUF includes SentencePiece's missing NFKC rules.
The public `transcribe_tokenize` now reaches this encoder.

The model owns an immutable token trie with failure links. Only trie-advancing
pieces get a bonus, and both the top-two margin and candidate-to-best distance
must be at most the configured margin. Blank winners cannot become speech.
Root paths start with a `▁` piece. Confidence retains upstream's distribution
confidence, not the chosen token's probability; do not use it as calibrated
confidence in the boosted word. TDT, offline RNN-T and streaming RNN-T call the same policy.
Streaming reconstructs the trie cursor from a bounded suffix of accumulated
tokens, preserving a prefix across encoder chunks and clearing it for a new
session. Offline meeting chunks use the same offline RNN-T loop.

This is greedy prefix biasing, **without shadow-state rollback**. If a boosted
prefix stops matching, its already-emitted pieces stay in the transcript.
Neither trie completion nor an acoustic margin proves that a whole word was
correct. Results must decide whether a more expensive rollback/beam design is
worth pursuing. CTC is untouched. TDT is hooked but not audio-evaluated here.

`spike/check-tokenizer.py` compares emitted IDs to the official SentencePiece
implementation using the GGUF's actual pieces, scores and types. Run it in a
temporary worker environment with `sentencepiece==0.2.1` and `protobuf`; pass the
GGUF and a runner JSONL. No tokenizer package is installed on Studio.

## Additional real-voice evaluation

`evaluate-real.py` takes a private corpus manifest and keeps the selected bonus
and margin frozen at 1.5. It checks source hashes, normalizes copies to mono
16 kHz PCM, then evaluates every clip offline and streaming. Run it through
HiveMind with the existing runner and model:

```sh
python3 spike/evaluate-real.py "$CARGO_TARGET_DIR/release/parakeet-boost-eval" \
  /path/to/parakeet-unified-en-0.6b-Q8_0.gguf perf/real-voice-expansion
```

The private manifest extends the normal clip fields with `sha256`, `origin`
(`dictation` or `meeting`), `source` (`microphone` or `system`) and
`reference_quality` (`history-proxy`, `final-proxy` or `rolling-proxy`). Optional
`corrected_entry_ids`, `known_live_hanks` and `collision_guard` label the
diagnostic cohorts. Its `vocabulary.txt` must exactly match the manifest's terms.
Every cohort, including diagnostics, separates origin, track and reference
quality. Corrected-Abattoir recall counts only Abattoir, once per clip; other
cohorts count distinct expected terms per clip. Diagnostic cohorts overlap
their parent cohorts and must not be added to them.
Source audio opens read-only; normalized copies, references and per-clip output
remain under the ignored corpus directory. Only `results/summary.json` is
suitable for a public aggregate receipt.

History and generated meeting transcripts are proxy references. Contextual
proper-name corrections must be recorded before examining candidate output;
they do not make the remaining words human-audited ground truth. Keep rolling,
final, microphone and system-track evidence separate when reporting results.

Rescore existing outputs against the exact manifest used for inference without
normalizing audio or running the engine again:

```sh
python3 spike/evaluate-real.py unused unused perf/real-voice-expansion --score-only
```

`real-results.json` is the checked-in count-only receipt for this follow-up.
Timing remains in the private summaries and does not establish a latency pass.

## Compare upstream English models

`model-runner` is a separate workspace pinned to unpatched `transcribe-cpp
0.4.1`. It compares Unified offline/streaming, Parakeet Ultra offline, and
Qwen3-ASR 1.7B offline with and without a context prompt. The phrase-boost
runner and production dependency keep their original pins.

```sh
# On the admitted Mac worker containing the normalized private corpus:
cargo build --release --locked -j 2 --manifest-path spike/model-runner/Cargo.toml
cargo clippy --release --locked -j 2 --all-targets --all-features \
  --manifest-path spike/model-runner/Cargo.toml -- -D warnings
python3 spike/compare-models.py "$CARGO_TARGET_DIR/release/hex-model-compare" \
  perf/real-voice-expansion
# Recompute aggregates without models or inference:
python3 spike/compare-models.py unused perf/real-voice-expansion --score-only
```

`comparison-models.json` pins Q8_0 artifact revisions and SHA-256 hashes. The
script verifies the existing normalized WAVs and downloads models only on the
worker. `--prepare-only` stops after checking artifacts and saving the plan.
Qwen's prompt is exactly `Names and technical terms: ` followed by the frozen
vocabulary, comma-separated, and a period. It contains no per-clip references,
transcripts or correction aliases. The runner checks `Feature::ContextPrompt`.

All three models remain resident on strict Metal, with one CPU thread and a
fresh native session per clip. Each clip rotates the five arms' order, shifted
again on each pass. One complete corpus pass warms every arm; three measured
passes follow. Accuracy uses pass one; the summary also counts transcript
changes in later passes. Timing includes session creation, decode and teardown,
but excludes model loading, file reads and offline minimum-length padding.
All arms request no timestamps and automatic language selection. Offline clips
have Hex's 1.5-second minimum padding; streams receive the original 20 ms blocks
as fast as computation permits. Streaming finalize time excludes earlier
compute and is not comparable to offline total time as a speed ranking.

This is a text-decoder comparison on preselected short crops. Multiple resident
models and shared-worker load differ from native dictation, and automatic
references can favor the engine that produced them. It cannot establish a
general English-default winner, a guard-gate pass, or shortcut-to-paste latency.
Old 0.2.4 boost results provide a historical accuracy comparison, not a controlled
timing comparison with this 0.4.1 runner.

Upstream sources: [prompting contract](https://github.com/handy-computer/transcribe.cpp/blob/v0.4.1/docs/prompting.md),
[Qwen card](https://github.com/handy-computer/transcribe.cpp/blob/v0.4.1/docs/models/qwen3-asr-1.7b.md)
(Apache-2.0), and [Moondream Parakeet Ultra card](https://github.com/handy-computer/transcribe.cpp/blob/v0.4.1/docs/models/parakeet-ultra.md)
(CC-BY-4.0). Model files and private outputs are excluded from git.
