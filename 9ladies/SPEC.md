# 9ladies — Image Description CLI

A command-line tool for batch image description using a VLM via llama.cpp HTTP API.

## Usage

```bash
ls *.jpg | 9ladies --prompt barcode-finder.json --url http://localhost:8080
```

Or with find:

```bash
find ./products -name "*.png" | 9ladies --prompt describe.json --url http://localhost:8080
```

## CLI Arguments

| Argument | Required | Description |
|----------|----------|-------------|
| `--prompt <file>` | Yes | Path to prompt configuration JSON file |
| `--url <url>` | Yes | llama.cpp server URL (e.g. `http://localhost:8080`) |
| `--dry-run` | No | Validate inputs without calling the model |

## Input

File paths read from stdin, one per line. Each path should point to an image file.

## Prompt File Format

JSON file with the following structure:

```json
{
  "system": "System prompt for the model",
  "prompt": "User prompt sent with each image",
  "temperature": 0.1
}
```

All fields required. Temperature should be a float between 0.0 and 2.0.

## Output

JSONL to stdout — one JSON object per line:

```json
{"file": "path/to/image.jpg", "response": <model output>}
```

The `response` field contains whatever the model returns. If the prompt asks for JSON, the model output will be a JSON object/string; otherwise plain text.

## Error Handling

- File not found, unreadable, or not a valid image: log to stderr, continue
- Prompt file invalid: exit with error before processing
- Model request fails: log to stderr, continue to next file
- Empty stdin: exit cleanly with no output

## Dry Run Mode

With `--dry-run`:

1. Parse and validate the prompt file
2. Check each input path exists and is readable
3. Log any issues to stderr
4. Do not contact the model server
5. Exit 0 if all valid, exit 1 if any issues found

## llama.cpp API

The tool should use the `/completion` or `/v1/chat/completions` endpoint (whichever is appropriate for vision models in the server setup). Details of the exact API shape to be confirmed during implementation — the user has an existing server with model loading/unloading that they'll describe further.

## Supported Image Formats

At minimum: JPEG, PNG, WebP, GIF. Detect by file content, not extension.
