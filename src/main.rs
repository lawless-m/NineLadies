use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use clap::Parser;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{self, BufRead};
use std::path::Path;
use std::process::ExitCode;
use std::time::Duration;

#[derive(Parser)]
#[command(name = "9ladies")]
#[command(about = "Batch image description tool using VLMs via Ollama")]
struct Args {
    /// Path to prompt configuration JSON file
    #[arg(long)]
    prompt: String,

    /// Server URL (e.g. http://localhost:8080 for llama.cpp, http://localhost:11434 for Ollama)
    #[arg(long)]
    url: String,

    /// Model name (required for Ollama, e.g. qwen2.5vl:32b or llava:13b)
    #[arg(long)]
    model: Option<String>,

    /// Validate inputs without calling the model
    #[arg(long)]
    dry_run: bool,
}

#[derive(Debug, Deserialize)]
struct PromptConfig {
    system: String,
    prompt: String,
    temperature: f32,
    #[serde(default)]
    model: Option<String>,
}

#[derive(Serialize)]
struct OutputRecord {
    file: String,
    response: serde_json::Value,
}

// Ollama native API types
#[derive(Serialize)]
struct OllamaChatRequest {
    model: String,
    messages: Vec<OllamaChatMessage>,
    stream: bool,
    options: OllamaOptions,
}

#[derive(Serialize)]
struct OllamaOptions {
    temperature: f32,
}

#[derive(Serialize)]
struct OllamaChatMessage {
    role: String,
    content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    images: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct OllamaChatResponse {
    message: OllamaMessageResponse,
}

#[derive(Deserialize)]
struct OllamaMessageResponse {
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
    if data.starts_with(b"RIFF") && &data[8..12] == b"WEBP" {
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
    model: &str,
    config: &PromptConfig,
    image_data: &[u8],
) -> Result<serde_json::Value, String> {
    let base64_image = BASE64.encode(image_data);

    let request = OllamaChatRequest {
        model: model.to_string(),
        messages: vec![
            OllamaChatMessage {
                role: "system".to_string(),
                content: config.system.clone(),
                images: None,
            },
            OllamaChatMessage {
                role: "user".to_string(),
                content: config.prompt.clone(),
                images: Some(vec![base64_image]),
            },
        ],
        stream: false,
        options: OllamaOptions {
            temperature: config.temperature,
        },
    };

    let url = format!("{}/api/chat", base_url.trim_end_matches('/'));

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

    let chat_response: OllamaChatResponse = response
        .json()
        .map_err(|e| format!("Failed to parse response: {}", e))?;

    let content = chat_response.message.content;

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

    // Model can come from CLI or prompt config
    let model = args.model.as_ref().or(config.model.as_ref());
    let model = match model {
        Some(m) => m.clone(),
        None => {
            eprintln!("Error: --model is required (or set 'model' in prompt config)");
            return ExitCode::from(1);
        }
    };

    // Read paths from stdin
    let stdin = io::stdin();
    let paths: Vec<String> = stdin.lock().lines().filter_map(|l| l.ok()).collect();

    if paths.is_empty() {
        return ExitCode::from(0);
    }

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .expect("Failed to create HTTP client");
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

        // Just validate format is recognized (already done in validate_image_file)
        if args.dry_run {
            continue;
        }

        // Call the model
        match call_model(&client, &args.url, &model, &config, &image_data) {
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

    if had_errors {
        ExitCode::from(1)
    } else {
        ExitCode::from(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixtures_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
    }

    // ==================== Image Format Detection Tests ====================

    #[test]
    fn test_detect_png() {
        let data = fs::read(fixtures_dir().join("red.png")).unwrap();
        assert_eq!(detect_image_format(&data), Some("png"));
    }

    #[test]
    fn test_detect_jpeg() {
        let data = fs::read(fixtures_dir().join("red.jpg")).unwrap();
        assert_eq!(detect_image_format(&data), Some("jpeg"));
    }

    #[test]
    fn test_detect_gif() {
        let data = fs::read(fixtures_dir().join("red.gif")).unwrap();
        assert_eq!(detect_image_format(&data), Some("gif"));
    }

    #[test]
    fn test_detect_webp() {
        let data = fs::read(fixtures_dir().join("red.webp")).unwrap();
        assert_eq!(detect_image_format(&data), Some("webp"));
    }

    #[test]
    fn test_detect_invalid_format() {
        let data = b"This is not an image file";
        assert_eq!(detect_image_format(data), None);
    }

    #[test]
    fn test_detect_too_short() {
        let data = b"short";
        assert_eq!(detect_image_format(data), None);
    }

    #[test]
    fn test_detect_empty() {
        let data: &[u8] = &[];
        assert_eq!(detect_image_format(data), None);
    }

    // ==================== Prompt Config Loading Tests ====================

    #[test]
    fn test_load_valid_prompt_config() {
        let path = fixtures_dir().join("test-prompt.json");
        let config = load_prompt_config(path.to_str().unwrap()).unwrap();

        assert_eq!(config.system, "You are a test assistant.");
        assert_eq!(config.prompt, "Describe this image.");
        assert!((config.temperature - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn test_load_invalid_prompt_config_missing_fields() {
        let path = fixtures_dir().join("invalid-prompt.json");
        let result = load_prompt_config(path.to_str().unwrap());

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Failed to parse"));
    }

    #[test]
    fn test_load_nonexistent_prompt_config() {
        let result = load_prompt_config("/nonexistent/path/config.json");

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Failed to read"));
    }

    #[test]
    fn test_load_prompt_config_invalid_temperature() {
        // Create a temp file with invalid temperature
        let temp_dir = std::env::temp_dir();
        let temp_file = temp_dir.join("invalid_temp_config.json");
        fs::write(&temp_file, r#"{"system": "test", "prompt": "test", "temperature": 3.0}"#).unwrap();

        let result = load_prompt_config(temp_file.to_str().unwrap());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Temperature must be between"));

        fs::remove_file(temp_file).ok();
    }

    // ==================== Image File Validation Tests ====================

    #[test]
    fn test_validate_png_image() {
        let path = fixtures_dir().join("red.png");
        let result = validate_image_file(&path);

        assert!(result.is_ok());
        let data = result.unwrap();
        assert!(!data.is_empty());
        assert_eq!(detect_image_format(&data), Some("png"));
    }

    #[test]
    fn test_validate_jpeg_image() {
        let path = fixtures_dir().join("red.jpg");
        let result = validate_image_file(&path);

        assert!(result.is_ok());
        let data = result.unwrap();
        assert_eq!(detect_image_format(&data), Some("jpeg"));
    }

    #[test]
    fn test_validate_gif_image() {
        let path = fixtures_dir().join("red.gif");
        let result = validate_image_file(&path);

        assert!(result.is_ok());
        let data = result.unwrap();
        assert_eq!(detect_image_format(&data), Some("gif"));
    }

    #[test]
    fn test_validate_webp_image() {
        let path = fixtures_dir().join("red.webp");
        let result = validate_image_file(&path);

        assert!(result.is_ok());
        let data = result.unwrap();
        assert_eq!(detect_image_format(&data), Some("webp"));
    }

    #[test]
    fn test_validate_nonexistent_file() {
        let path = Path::new("/nonexistent/image.png");
        let result = validate_image_file(path);

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("File not found"));
    }

    #[test]
    fn test_validate_non_image_file() {
        let path = fixtures_dir().join("not-an-image.txt");
        let result = validate_image_file(&path);

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Not a valid image format"));
    }

    // ==================== Output Record Serialization Tests ====================

    #[test]
    fn test_output_record_with_string_response() {
        let record = OutputRecord {
            file: "test.jpg".to_string(),
            response: serde_json::Value::String("A red image".to_string()),
        };

        let json = serde_json::to_string(&record).unwrap();
        assert!(json.contains("\"file\":\"test.jpg\""));
        assert!(json.contains("\"response\":\"A red image\""));
    }

    #[test]
    fn test_output_record_with_json_response() {
        let record = OutputRecord {
            file: "test.jpg".to_string(),
            response: serde_json::json!({"barcode": true, "ingredients": false}),
        };

        let json = serde_json::to_string(&record).unwrap();
        assert!(json.contains("\"file\":\"test.jpg\""));
        assert!(json.contains("\"barcode\":true"));
    }

    // ==================== Ollama Request Serialization Tests ====================

    #[test]
    fn test_ollama_request_serialization() {
        let request = OllamaChatRequest {
            model: "qwen2.5vl:32b".to_string(),
            messages: vec![
                OllamaChatMessage {
                    role: "system".to_string(),
                    content: "You are helpful.".to_string(),
                    images: None,
                },
                OllamaChatMessage {
                    role: "user".to_string(),
                    content: "Describe this.".to_string(),
                    images: Some(vec!["abc123".to_string()]),
                },
            ],
            stream: false,
            options: OllamaOptions { temperature: 0.7 },
        };

        let json = serde_json::to_string(&request).unwrap();

        // Verify structure
        assert!(json.contains("\"model\":\"qwen2.5vl:32b\""));
        assert!(json.contains("\"role\":\"system\""));
        assert!(json.contains("\"role\":\"user\""));
        assert!(json.contains("\"images\":[\"abc123\"]"));
        assert!(json.contains("\"stream\":false"));
        assert!(json.contains("\"temperature\":0.7"));
    }

    // ==================== Integration-style Tests ====================

    #[test]
    fn test_full_validation_pipeline_with_known_images() {
        let fixtures = fixtures_dir();
        let prompt_path = fixtures.join("test-prompt.json");

        // Load config
        let config = load_prompt_config(prompt_path.to_str().unwrap()).unwrap();
        assert_eq!(config.system, "You are a test assistant.");

        // Validate all test images
        let test_images = ["red.png", "red.jpg", "red.gif", "red.webp"];

        for image_name in test_images {
            let image_path = fixtures.join(image_name);
            let data = validate_image_file(&image_path).unwrap();
            let format = detect_image_format(&data).unwrap();

            // Verify we can encode to base64 for API call
            let encoded = BASE64.encode(&data);
            assert!(!encoded.is_empty());

            // Verify data URL format
            let data_url = format!("data:image/{};base64,{}", format, encoded);
            assert!(data_url.starts_with("data:image/"));
        }
    }

    #[test]
    fn test_dry_run_detects_invalid_files() {
        let fixtures = fixtures_dir();

        // Valid image should pass
        let valid_path = fixtures.join("red.png");
        assert!(validate_image_file(&valid_path).is_ok());

        // Invalid image should fail
        let invalid_path = fixtures.join("not-an-image.txt");
        assert!(validate_image_file(&invalid_path).is_err());

        // Nonexistent file should fail
        let missing_path = fixtures.join("does-not-exist.png");
        assert!(validate_image_file(&missing_path).is_err());
    }
}
