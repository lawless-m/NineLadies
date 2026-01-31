# Contents

Specification package for **9ladies**, a batch image description CLI tool.

## Start Here

1. **SPEC.md** — full specification for the tool (CLI args, input/output format, error handling)
2. **README.md** — overview and quick start examples

## Prompt Examples

In `prompts/`:

- `barcode-finder.json` — structured detection of barcodes and ingredients
- `people-count.json` — count people in photographs  
- `describe.json` — general image description

These are examples to get started; the tool accepts any prompt file matching the format in SPEC.md.

## For Implementation

The spec intentionally leaves the llama.cpp API details loose — the user has an existing server setup with model loading/unloading that they'll describe when implementing. The core contract is: read paths from stdin, load prompt from file, call the model, emit JSONL.
