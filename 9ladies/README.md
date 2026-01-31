# 9ladies

Batch image description tool using VLMs via llama.cpp.

Named after the Nine Ladies stone circle in Derbyshire.

## Quick Start

```bash
# Describe images, output JSONL
ls photos/*.jpg | 9ladies --prompt prompts/describe.json --url http://localhost:8080

# Check for barcodes in product images  
find ./products -name "*.png" | 9ladies --prompt prompts/barcode-finder.json --url http://localhost:8080

# Validate without hitting the model
ls *.jpg | 9ladies --prompt prompts/describe.json --url http://localhost:8080 --dry-run
```

## Files

- `SPEC.md` — full specification
- `prompts/` — example prompt files
  - `barcode-finder.json` — detect barcodes and ingredients lists
  - `people-count.json` — count people in photos
  - `describe.json` — general image description

## Building

Rust project. Details TBD during implementation.

## Model Server

Expects a llama.cpp server running with a vision model loaded (e.g. Qwen2.5-VL 7B). The user has an existing model loading/unloading system — details to be provided during implementation.
