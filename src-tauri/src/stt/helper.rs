//! Out-of-process SenseVoice decode helper.

use super::*;

/// SenseVoice occasionally emits inline metadata tokens such as
/// `<|ja|><|NEUTRAL|><|Speech|><|withitn|>` at the start of decoded text
/// (and sometimes mid-output between utterances). Strip anything inside
/// `<|...|>` so users and the downstream AI summariser never see them.
fn strip_sense_voice_tags(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'<' && i + 1 < bytes.len() && bytes[i + 1] == b'|' {
            // Find the matching "|>" closing marker.
            if let Some(rel) = text[i + 2..].find("|>") {
                i += 2 + rel + 2;
                continue;
            }
        }
        // UTF-8-safe copy by char boundary: advance one char.
        let ch_end = text[i..]
            .char_indices()
            .nth(1)
            .map(|(n, _)| i + n)
            .unwrap_or(bytes.len());
        out.push_str(&text[i..ch_end]);
        i = ch_end;
    }
    out.trim().to_string()
}

pub(super) fn decode_samples(
    recognizer: &OfflineRecognizer,
    sample_rate: i32,
    samples: &[f32],
) -> String {
    if samples.is_empty() {
        return String::new();
    }
    let stream = recognizer.create_stream();
    stream.accept_waveform(sample_rate, samples);
    recognizer.decode(&stream);
    stream
        .get_result()
        .map(|r| strip_sense_voice_tags(r.text.trim()))
        .unwrap_or_default()
}

fn read_f32_samples(path: &Path) -> Result<Vec<f32>, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("STT helper sample read failed: {e}"))?;
    if !bytes.len().is_multiple_of(4) {
        return Err("STT helper sample file is not aligned to f32".into());
    }
    f32_samples_from_bytes(&bytes)
}

fn f32_samples_from_bytes(bytes: &[u8]) -> Result<Vec<f32>, String> {
    if !bytes.len().is_multiple_of(4) {
        return Err("STT helper sample payload is not aligned to f32".into());
    }
    Ok(bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect())
}

fn f32_samples_from_request(request: &SttHelperDecodeRequest) -> Result<Vec<f32>, String> {
    if !request.sample_data.is_empty() {
        let bytes = BASE64_STANDARD
            .decode(&request.sample_data)
            .map_err(|e| format!("STT helper sample payload decode failed: {e}"))?;
        f32_samples_from_bytes(&bytes)
    } else {
        read_f32_samples(Path::new(&request.samples))
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct SttHelperDecodeRequest {
    sample_rate: i32,
    #[serde(default)]
    samples: String,
    #[serde(default)]
    sample_data: String,
    #[serde(default)]
    shutdown: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct SttHelperDecodeResponse {
    ready: bool,
    ok: bool,
    text: String,
    error: String,
}

fn write_helper_response(response: &SttHelperDecodeResponse) -> Result<(), String> {
    let mut stdout = std::io::stdout();
    serde_json::to_writer(&mut stdout, response)
        .map_err(|e| format!("STT helper response serialization failed: {e}"))?;
    stdout
        .write_all(b"\n")
        .map_err(|e| format!("STT helper response write failed: {e}"))?;
    stdout
        .flush()
        .map_err(|e| format!("STT helper response flush failed: {e}"))
}

fn stt_helper_ready_response() -> SttHelperDecodeResponse {
    SttHelperDecodeResponse {
        ready: true,
        ok: true,
        text: String::new(),
        error: String::new(),
    }
}

fn stt_helper_error_response(error: impl Into<String>) -> SttHelperDecodeResponse {
    SttHelperDecodeResponse {
        ready: false,
        ok: false,
        text: String::new(),
        error: error.into(),
    }
}

fn stt_helper_decode_response(text: String) -> SttHelperDecodeResponse {
    SttHelperDecodeResponse {
        ready: false,
        ok: true,
        text,
        error: String::new(),
    }
}

fn arg_value(args: &[String], name: &str) -> Option<String> {
    args.windows(2)
        .find(|pair| pair[0] == name)
        .map(|pair| pair[1].clone())
}

fn helper_recognizer_config_from_args(args: &[String]) -> Result<OfflineRecognizerConfig, String> {
    let model_path = PathBuf::from(arg_value(args, "--model").ok_or("missing --model")?);
    let tokens_path = PathBuf::from(arg_value(args, "--tokens").ok_or("missing --tokens")?);
    let language = arg_value(args, "--language").unwrap_or_else(|| "ja".into());
    let provider_arg = arg_value(args, "--provider").unwrap_or_else(|| STT_BACKEND_CPU.into());
    let provider = provider_arg;

    let mut config = OfflineRecognizerConfig::default();
    config.model_config.sense_voice = OfflineSenseVoiceModelConfig {
        model: Some(model_path.to_string_lossy().into_owned()),
        language: Some(language),
        use_itn: true,
    };
    config.model_config.tokens = Some(tokens_path.to_string_lossy().into_owned());
    config.model_config.provider = Some(provider);
    config.model_config.num_threads = 4;
    config.decoding_method = Some("greedy_search".into());
    Ok(config)
}

fn run_decode_server_from_args(args: &[String]) -> i32 {
    let result: Result<(), String> = (|| {
        let config = helper_recognizer_config_from_args(args)?;
        let recognizer = match OfflineRecognizer::create(&config) {
            Some(recognizer) => recognizer,
            None => {
                let _ = write_helper_response(&stt_helper_error_response(
                    "failed to create STT helper recognizer",
                ));
                return Ok(());
            }
        };
        write_helper_response(&stt_helper_ready_response())?;

        let stdin = std::io::stdin();
        for line in stdin.lock().lines() {
            let line = line.map_err(|e| format!("STT helper request read failed: {e}"))?;
            if line.trim().is_empty() {
                continue;
            }
            let request: SttHelperDecodeRequest = serde_json::from_str(&line)
                .map_err(|e| format!("STT helper request parse failed: {e}"))?;
            if request.shutdown {
                break;
            }
            let response = match f32_samples_from_request(&request) {
                Ok(samples) => {
                    let text = decode_samples(&recognizer, request.sample_rate, &samples);
                    stt_helper_decode_response(text)
                }
                Err(err) => stt_helper_error_response(err),
            };
            write_helper_response(&response)?;
        }
        Ok(())
    })();

    match result {
        Ok(()) => 0,
        Err(err) => {
            let _ = write_helper_response(&stt_helper_error_response(err.clone()));
            eprintln!("{err}");
            1
        }
    }
}

pub fn run_decode_helper_from_args() -> Option<i32> {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|arg| arg == STT_DECODE_HELPER_ARG) {
        return Some(run_decode_server_from_args(&args));
    }
    if !args.iter().any(|arg| arg == STT_DECODE_HELPER_ARG) {
        return None;
    }

    let result: Result<(), String> = (|| {
        let sample_rate = arg_value(&args, "--sample-rate")
            .and_then(|value| value.parse::<i32>().ok())
            .unwrap_or(TARGET_SAMPLE_RATE);
        let samples_path = PathBuf::from(arg_value(&args, "--samples").ok_or("missing --samples")?);
        let samples = read_f32_samples(&samples_path)?;

        let config = helper_recognizer_config_from_args(&args)?;
        let recognizer =
            OfflineRecognizer::create(&config).ok_or("failed to create STT helper recognizer")?;
        let text = decode_samples(&recognizer, sample_rate, &samples);
        println!("{text}");
        Ok(())
    })();

    match result {
        Ok(()) => Some(0),
        Err(err) => {
            eprintln!("{err}");
            Some(1)
        }
    }
}
