//! Typed microphone callbacks for every sample format exposed by CPAL.

use cpal::traits::DeviceTrait;
use std::sync::mpsc;

pub(super) fn build_input_stream(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    format: cpal::SampleFormat,
    audio_tx: mpsc::Sender<Vec<f32>>,
    on_error: impl FnMut(cpal::StreamError) + Send + 'static,
) -> Result<cpal::Stream, String> {
    macro_rules! converted_stream {
        ($sample:ty) => {
            device.build_input_stream(
                config,
                move |data: &[$sample], _| {
                    let _ = audio_tx.send(super::super::normalize_input(data));
                },
                on_error,
                None,
            )
        };
    }
    let stream = match format {
        // Keep the existing direct copy for the usual floating-point input.
        cpal::SampleFormat::F32 => device.build_input_stream(
            config,
            move |data: &[f32], _| {
                let _ = audio_tx.send(data.to_vec());
            },
            on_error,
            None,
        ),
        cpal::SampleFormat::I8 => converted_stream!(i8),
        cpal::SampleFormat::I16 => converted_stream!(i16),
        cpal::SampleFormat::I32 => converted_stream!(i32),
        cpal::SampleFormat::I64 => converted_stream!(i64),
        cpal::SampleFormat::U8 => converted_stream!(u8),
        cpal::SampleFormat::U16 => converted_stream!(u16),
        cpal::SampleFormat::U32 => converted_stream!(u32),
        cpal::SampleFormat::U64 => converted_stream!(u64),
        cpal::SampleFormat::F64 => converted_stream!(f64),
        other => return Err(format!("未対応の音声フォーマットです: {:?}", other)),
    };
    stream.map_err(|err| format!("マイクストリーム開始失敗: {}", err))
}
