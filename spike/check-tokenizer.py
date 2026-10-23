#!/usr/bin/env python3
"""Compare the spike's token IDs to official SentencePiece on the GGUF vocab.

Run in a temporary worker venv with sentencepiece and protobuf. No audio use.
The GGUF lacks a normalizer model; this checks the supported ASCII phrase scope.
"""
import argparse
import json
import struct
import sentencepiece as spm
from sentencepiece import sentencepiece_model_pb2 as proto

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("model")
parser.add_argument("results")
args = parser.parse_args()
metadata = {}
with open(args.model, "rb") as f:
    assert f.read(4) == b"GGUF"
    version, tensors, count = struct.unpack("<IQQ", f.read(20))

    def string():
        return f.read(struct.unpack("<Q", f.read(8))[0]).decode()

    def value(kind):
        if kind == 8:
            return string()
        if kind == 9:
            element, length = struct.unpack("<IQ", f.read(12))
            return [value(element) for _ in range(length)]
        fmt = {0: "B", 1: "b", 2: "H", 3: "h", 4: "I", 5: "i", 6: "f", 7: "?", 10: "Q", 11: "q", 12: "d"}[kind]
        return struct.unpack("<" + fmt, f.read(struct.calcsize(fmt)))[0]

    for _ in range(count):
        key = string()
        item = value(struct.unpack("<I", f.read(4))[0])
        if key.startswith("tokenizer."):
            metadata[key] = item

model = proto.ModelProto()
model_type = metadata["tokenizer.ggml.model"]
model.trainer_spec.model_type = {"bpe": proto.TrainerSpec.BPE, "unigram": proto.TrainerSpec.UNIGRAM}[model_type]
model.trainer_spec.unk_id = 0
model.trainer_spec.bos_id = -1
model.trainer_spec.eos_id = -1
model.normalizer_spec.name = "identity"
for token, score, kind in zip(metadata["tokenizer.ggml.tokens"], metadata["tokenizer.ggml.scores"], metadata["tokenizer.ggml.token_type"]):
    piece = model.pieces.add()
    piece.piece, piece.score, piece.type = token, score, kind
processor = spm.SentencePieceProcessor(model_proto=model.SerializeToString())
checked = 0
for line in open(args.results):
    row = json.loads(line)
    if row["type"] != "tokenization":
        continue
    expected = processor.encode(row["term"], out_type=int)
    assert expected == row["ids"], (row["term"], expected, row["ids"])
    checked += 1
assert checked > 0
print(f"Official SentencePiece ({model_type}) agrees on all {checked} phrase tokenizations")
