use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use clap::Parser;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{self, BufRead};
use std::path::Path;
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "9ladies")]
#[command(about = "Batch image description tool using VLMs via llama.cpp")]
struct Args {
    /// Path to prompt configuration JSON file
    #[arg(long)]
    prompt: String,

    /// llama.cpp server URL (e.g. http://localhost:8080)
    #[arg(long)]
    url: String,

    /// Validate inputs without calling the model
    #[arg(long)]
    dry_run: bool,
}

#[derive(Deserialize)]
struct PromptConfig {
    system: String,
    prompt: String,
    temperature: f32,
}

#[derive(Serialize)]
struct OutputRecord {
    file: String,
    response: serde_json::Value,
}

#[derive(Serialize)]
struct ChatRequest {
    messages: Vec<ChatMessage>,
    temperature: f32,
}

#[derive(Serialize)]
struct ChatMessage {
    role: String,
    content: Vec<ContentPart>,
}

#[derive(Serialize)]
#[serde(tag = "type")]
enum ContentPart {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image_url")]
    ImageUrl { image_url: ImageUrlData },
}

#[derive(Serialize)]
struct ImageUrlData {
    url: String,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatMessageResponse,
}

#[derive(Deserialize)]
struct ChatMessageResponse {
    content: String,
}

fn detect_image_format(data: &[u8]) -> Option<&'static str> {
    if data.len() < 12 {
        return None;
    }

    // JPEG: starts with FF D8 FF
    if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some("jpeg");
    }

    // PNG: starts with 89 50 4E 47 0D 0A 1A 0A
    if data.starts_with(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]) {
        return Some("png");
    }

    // GIF: starts with GIF87a or GIF89a
    if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
        return Some("gif");
    }

    // WebP: starts with RIFF....WEBP
    if data.len() >= 12 && data.starts_with(b"RIFF") && &data[8..12] == b"WEBP" {
        return Some("webp");
    }

    None
}

fn load_prompt_config(path: &str) -> Result<PromptConfig, String> {
    let content = fs::read_to_string(path)
        .map_err(|e| format!("Failed to read prompt file '{}': {}", path, e))?;

    let config: PromptConfig = serde_json::from_str(&content)
        .map_err(|e| format!("Failed to parse prompt file '{}': {}", path, e))?;

    if config.temperature < 0.0 || config.temperature > 2.0 {
        return Err(format!(
            "Temperature must be between 0.0 and 2.0, got {}",
            config.temperature
        ));
    }

    Ok(config)
}

fn validate_image_file(path: &Path) -> Result<Vec<u8>, String> {
    if !path.exists() {
        return Err(format!("File not found: {}", path.display()));
    }

    let data = fs::read(path).map_err(|e| format!("Cannot read file '{}': {}", path.display(), e))?;

    if detect_image_format(&data).is_none() {
        return Err(format!(
            "Not a valid image format (expected JPEG, PNG, WebP, or GIF): {}",
            path.display()
        ));
    }

    Ok(data)
}

fn call_model(
    client: &reqwest::blocking::Client,
    base_url: &str,
    config: &PromptConfig,
    image_data: &[u8],
    image_format: &str,
) -> Result<serde_json::Value, String> {
    let base64_image = BASE64.encode(image_data);
    let data_url = format!("data:image/{};base64,{}", image_format, base64_image);

    let request = ChatRequest {
        messages: vec![
            ChatMessage {
                role: "system".to_string(),
                content: vec![ContentPart::Text {
                    text: config.system.clone(),
                }],
            },
            ChatMessage {
                role: "user".to_string(),
                content: vec![
                    ContentPart::ImageUrl {
                        image_url: ImageUrlData { url: data_url },
                    },
                    ContentPart::Text {
                        text: config.prompt.clone(),
                    },
                ],
            },
        ],
        temperature: config.temperature,
    };

    let url = format!("{}/v1/chat/completions", base_url.trim_end_matches('/'));

    let response = client
        .post(&url)
        .json(&request)
        .send()
        .map_err(|e| format!("Request failed: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().unwrap_or_default();
        return Err(format!("Server returned {}: {}", status, body));
    }

    let chat_response: ChatResponse = response
        .json()
        .map_err(|e| format!("Failed to parse response: {}", e))?;

    let content = chat_response
        .choices
        .first()
        .map(|c| c.message.content.clone())
        .unwrap_or_default();

    // Try to parse as JSON, otherwise return as string
    match serde_json::from_str::<serde_json::Value>(&content) {
        Ok(json) => Ok(json),
        Err(_) => Ok(serde_json::Value::String(content)),
    }
}

fn main() -> ExitCode {
    let args = Args::parse();

    // Load and validate prompt config first
    let config = match load_prompt_config(&args.prompt) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Error: {}", e);
            return ExitCode::from(1);
        }
    };

    // Read paths from stdin
    let stdin = io::stdin();
    let paths: Vec<String> = stdin.lock().lines().filter_map(|l| l.ok()).collect();

    if paths.is_empty() {
        return ExitCode::from(0);
    }

    let client = reqwest::blocking::Client::new();
    let mut had_errors = false;

    for path_str in paths {
        let path_str = path_str.trim();
        if path_str.is_empty() {
            continue;
        }

        let path = Path::new(path_str);

        // Validate the image file
        let image_data = match validate_image_file(path) {
            Ok(data) => data,
            Err(e) => {
                eprintln!("{}", e);
                had_errors = true;
                continue;
            }
        };

        let format = detect_image_format(&image_data).unwrap(); // Safe: validated above

        if args.dry_run {
            // In dry-run mode, just validate (already done above)
            continue;
        }

        // Call the model
        match call_model(&client, &args.url, &config, &image_data, format) {
            Ok(response) => {
                let record = OutputRecord {
                    file: path_str.to_string(),
                    response,
                };
                println!("{}", serde_json::to_string(&record).unwrap());
            }
            Err(e) => {
                eprintln!("Error processing '{}': {}", path_str, e);
                had_errors = true;
            }
        }
    }

    if args.dry_run && had_errors {
        ExitCode::from(1)
    } else {
        ExitCode::from(0)
    }
}
