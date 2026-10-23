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
