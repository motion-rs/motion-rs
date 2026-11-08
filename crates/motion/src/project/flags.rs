//! Value parsers for time and codec options.

use super::invalid;
use crate::{Result, Time};

pub(super) fn parse_time(value: &str) -> Result<Time> {
    let (value, factor) = if let Some(value) = value.strip_suffix("ms") {
        (value, 0.001)
    } else if let Some(value) = value.strip_suffix('s') {
        (value, 1.0)
    } else {
        (value, 1.0)
    };
    let seconds = value
        .parse::<f64>()
        .map_err(|error| invalid(error.to_string()))?
        * factor;
    if !seconds.is_finite() || !(0.0..=86400.0).contains(&seconds) {
        return Err(invalid("Time must be finite and within 0..=86400 seconds"));
    }
    Ok(Time::from_seconds(seconds))
}

pub(super) fn parse_codec(
    name: &str,
    quality: Option<u8>,
    alpha: bool,
    rate: crate::FrameRate,
) -> Result<crate::Codec> {
    if quality.is_some()
        && matches!(
            name,
            "prores" | "prores4444" | "png" | "png-sequence" | "gif"
        )
    {
        return Err(invalid("Selected codec does not accept a quality setting"));
    }
    let quality = quality.unwrap_or(18);
    let codec = match name {
        "h264" if !alpha => crate::Codec::H264 {
            quality,
            preset: crate::EncoderPreset::Medium,
        },
        "h265" | "hevc" if !alpha => crate::Codec::H265 { quality },
        "vp9" => crate::Codec::Vp9 { quality, alpha },
        "prores" | "prores4444" => crate::Codec::ProRes4444,
        "png" | "png-sequence" => crate::Codec::PngSequence,
        "gif" => crate::Codec::Gif {
            frame_rate: rate,
            colors: 256,
        },
        "h264" | "h265" | "hevc" => return Err(invalid("Selected codec does not support alpha")),
        _ => return Err(invalid(format!("Unknown codec: {name}"))),
    };
    codec.validate()?;
    Ok(codec)
}
