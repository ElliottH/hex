# Keep the phrase boost experimental

The tested greedy phrase boost does not clear the shipping gate. It improves
synthetic names, including held-out sentences, but both original Abattoir misses
still decode as Avatar. An expanded real-meeting evaluation recovers few of the
additional Abattoir misses. The located Hanks miss belongs to a live draft;
both tested Parakeet configurations already recognize Hex in that clip. Keep
Hex on its released upstream engine and retain this patch for research; carrying
a production fork is not justified by these measurements.

## Synthetic recall improves; the motivating real miss remains

Measured on the M4 Pro mini with strict Metal, `transcribe-cpp` 0.2.4 and the
pinned `parakeet_unified_en` model. The selected bonus and acoustic margin are
both 1.5. They were selected from nine combinations on 34 tuning sentences,
before inspecting held-out sentences or guards. The lowest bonus, then lowest
margin, breaks recall ties. A uniform bonus makes these combinations only three
distinct effective flip thresholds: a candidate must be within the margin and
strictly less than the bonus behind the winner. This sweep does not establish
independent effects for bonus and margin.

| Offline metric | Baseline | Selected boost |
| --- | ---: | ---: |
| Tuning recall, 34 sentences | 31/34 (91.18%) | 34/34 (100%) |
| Held-out recall, 17 sentences | 13/17 (76.47%) | 15/17 (88.24%) |
| All synthetic target recall | 44/51 (86.27%) | 49/51 (96.08%) |
| Private target recall | 3/5 (60%) | 3/5 (60%) |
| All target recall, including private | 47/56 (83.93%) | 52/56 (92.86%) |
| Combined WER, 5,322 words | 103 errors (1.935%) | 98 errors (1.841%) |
| Guard WER, 4,717 words | 77 errors (1.632%) | 78 errors (1.654%) |
| Guard clips with excess vocabulary | 0/218 | 1/218 (0.459%) |
| Distractors with excess vocabulary | 0/18 | 1/18 (5.56%) |
| Ordinary-speech excess vocabulary | 0/200 | 0/200 |

The held-out gains are `OpenPode` to `OpenCode` and `Scroll` to `Scrawl`.
The distractor regression is `toolkit` to `tool kit`: the word-boundary scorer
counts the newly separate `kit` as an excess occurrence of `Kit`. This is a
spacing regression, not a demonstrated Tom Hanks-to-Hex or Avatar-to-Abattoir
false replacement. Both of those legitimate distractor names stay intact.
The combined guard passes the plan's 0.5% threshold numerically, but the small
distractor set does not support a near-zero-regression claim.

Streaming uses 84 clips: all 51 targets, 18 distractors, five private clips and
the first ten public ordinary clips. Target recall again rises from 47/56 to
52/56, and private recall stays at 3/5. Guard WER rises from 6/339 (1.770%) to
9/339 (2.655%), a **0.885 percentage-point regression**, exceeding the 0.3-point
guard limit. The same `toolkit` split occurs. The ten ordinary streaming clips
are only a smoke subset, not the 200-clip offline guard.

The meeting-sized replay packs the 69 synthetic sentences into 11
sentence-aligned chunks with 500 ms pauses, each at most 20 seconds. Offline
RNN-T recall rises from 20/21 to 21/21 distinct term-per-chunk expectations;
combined WER falls from 12/462 (2.60%) to 8/462 (1.73%). Distractor WER rises
from 3/131 to 5/131, with one excess vocabulary occurrence. These chunks reuse
tuning audio, so they establish that the decoder path consumes the boost, not
independent generalization. They do not exercise Hex's pause detector,
overlap deduplication, echo suppression, or native meeting scheduler.

## More real audio confirms that Abattoir remains difficult

The follow-up evaluates 168 private clips totaling 2,207 seconds: 20 retained
dictations, 131 meeting clips with final-transcript references, and 17 with
rolling-transcript references. All 11 available meetings are represented.
Bonus and margin stay frozen at 1.5; none of these clips selected parameters.
Original recordings were read-only. Only aggregate counts and hashes are
committed in [`spike/real-results.json`](../../spike/real-results.json).

References are automatic history or meeting transcripts, with 16 contextual
Avatar-to-Abattoir corrections across 15 clips recorded before inference.
These corrections are not a human audit of the surrounding words. Selection
favored vocabulary-bearing passages, known misses and a few term-free guards;
the sample does not estimate everyday accuracy. Crops follow transcript
timestamps with bounded padding, which can include or omit neighboring words.
The following word-error counts therefore measure **proxy disagreement**.

| Final-reference cohort | Recall, baseline → boost | Proxy word errors, baseline → boost | Excess vocabulary occurrences |
| --- | ---: | ---: | ---: |
| Microphone, offline (69 clips) | 59/74 → 62/74 | 158 → 166 / 3,054 words | 1 → 3 |
| Microphone, streaming | 62/74 → 64/74 | 206 → 215 / 3,054 words | 0 → 2 |
| System, offline (62 clips) | 60/66 → 61/66 | 117 → 122 / 2,150 words | 1 → 2 |
| System, streaming | 61/66 → 62/66 | 137 → 139 / 2,150 words | 2 → 2 |

The microphone proxy-disagreement rate increases by 0.262 percentage points
offline and 0.295 streaming. These vocabulary-heavy cohorts do not measure
the guard WER gate, and their reference text has not been human-audited.
The new excess occurrences are Scrawl and Dax. Both Scrawl passages discuss
the app, so the proxy's “scroll” is likely wrong; the Dax reference also has
an uncertain name. These counts do not establish false replacements. Frozen
references preserve the comparison rather than retroactively crediting gains.

Abattoir-only recall in the corrected subset is much lower than overall recall.
Each clip contributes one expectation even when it contains repeated mentions:

| Corrected subset | Offline baseline → boost | Streaming baseline → boost |
| --- | ---: | ---: |
| Microphone, 11 clips | 0/11 → 1/11 | 2/11 → 3/11 |
| System, 4 clips | 0/4 → 0/4 | 0/4 → 1/4 |

The Hanks clip already says Hex in the final transcript and in all four runs.
It demonstrates a live-draft/final-decoder difference, not a boost recovery.
The separate literal Avatar guard retains Avatar offline in both runs;
streaming drops Avatar in both. Neither inserts Abattoir. One short,
timestamp-cropped guard cannot establish collision safety.

All 20 dictations are term-free. Their proxy errors stay at 13/392 words
offline and 1/392 streaming, with zero excess vocabulary. The rolling-reference
meeting remains exploratory: microphone recall stays 10/10 in both modes,
while system recall stays 7/7 offline and rises 6/7 → 7/7 streaming. Microphone
proxy errors are 6 → 7 / 266 words offline and 11 → 11 streaming; system errors
stay 16/183 offline and 14/183 streaming. No rolling clip has excess vocabulary.

## Timing does not establish a latency pass

The initial comparisons have one whole-corpus warmup and five measured passes.
Offline p95 compute time rises from 177.44 to 246.61 ms (+69.18 ms); median rises
from 68.12 to 85.44 ms. Streaming total-compute p95 falls from 645.40 to 539.14 ms
and finalize p95 falls from 81.97 to 68.92 ms. Other worker jobs ran during
these sequential passes. The inconsistent direction and changing load prevent
attributing these differences to the boost.

The baseline is the patched binary with bonus disabled. It still loads the trie
and reconstructs a bounded token suffix, so this comparison does not measure
the patch's full overhead against pristine upstream. Streams receive audio as
fast as compute permits, without real-time sleeps. Neither shortcut-to-paste
latency nor concurrent meeting/dictation latency was measured. The required
no-measurable-latency-hit gate is **unproven**, not passed.
The expanded real-audio runs use one whole-corpus warmup, three measured
offline passes and one streaming pass. They share those timing limitations
and do not exercise the native meeting scheduler or shortcut-to-paste path.
Accuracy metrics use the first measured transcript pass; additional passes
contribute timing only.

## The corpus separates public fixtures from private evidence

The committed corpus has 69 ElevenLabs sentences: three sentences for each of
17 names and 18 real-word distractors. Argon, Astra and HiveMind came from
recent Hex history alongside the original names. The public guard has 200
LibriSpeech `test-clean` clips. The combined offline manifest has 274 clips.
See the [fixture guide](../../tests/fixtures/vocabulary/README.md) for generation,
hashes, voice/model settings and public-download instructions.

History returned 2,000 entries from the saved seven-day retention window, despite
requesting 30 days. Every retained raw/final pair was identical, so history did
not provide corrected ground truth. Five timestamp-matched diagnostic WAVs
were retained outside git. Their references use history plus context-inferred
proper-name corrections, and have not been human-audited. Two contain the
Abattoir/Avatar miss; neither improves. The later meeting extraction described
above supplies the Hanks clip and more Abattoir examples. Real Abattoir recall
remains below the 70% target, and human-audited accuracy remains incomplete.

Scoring ignores case and internal spacing for term recall, but requires whole
word boundaries; `open code` counts as OpenCode, while `toolkit` does not count
as Kit. WER uses case-insensitive alphanumeric words and retains word boundaries.
False replacements count vocabulary occurrences beyond the reference count;
the rate is the fraction of affected clips. These are text metrics, not an
alignment-based claim that the engine replaced a particular spoken word.

## The patch tests a narrower design than the plan

[`spike/engine.patch`](../../spike/engine.patch) adds score-ranked SentencePiece
BPE/unigram phrase encoding, a token trie with failure links, and one shared
selection rule in TDT, offline RNN-T and streaming RNN-T. Blank winners stay
blank; both top-two uncertainty and candidate-to-best acoustic distance must
fit the configured margin. Confidence remains upstream's distribution score,
not the probability of the boosted token.

The encoder supports ASCII phrases only because GGUF omits SentencePiece's
normalization rules. Official SentencePiece, using this GGUF's actual BPE type,
agreed on all 37 distinct original/lowercase/initial-capital phrase encodings.
C++ checks cover root/continuation pieces, blank
protection, acoustic-distance gating and disabled boosting. Unified RNN-T was
audio-evaluated; TDT was only compiled. The patch omits the plan's shadow-state
rollback, so an unsuccessful boosted prefix can remain in the output.

This experiment deliberately uses shipped 0.2.4 rather than the planned 0.4.1
upgrade, which keeps engine-version changes out of the comparison. Production
dependencies are unchanged. No fork or upstream PR was published, and no app
was installed, restarted, or tested with live audio devices.

The isolated runner built and passed strict all-target/all-feature release
Clippy on mini. Its toolchain emitted a debug-strip warning because
`rust-objcopy` could not load `libLLVM.dylib`; compilation and execution succeeded.
The generated engine matched the checksum-verified crate plus the recorded
patch. Independent Opus 5.5 medium review returned no blockers after fixing
held-out reporting, tokenizer-oracle coverage and source/vocabulary verification.

Reproduction commands are in [`spike/README.md`](../../spike/README.md).
[`spike/results.json`](../../spike/results.json) contains aggregate metrics and
raw-result hashes only. Private WAVs, references and transcript outputs stay
in the ignored local corpus. A production bridge should wait for improved
real-audio results, streaming guards and an uncontended pristine-engine timing
comparison, preferably through upstream vocabulary support.
