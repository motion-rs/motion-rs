//! Bounded SRT interchange and stateless, half-open caption sampling.

use motion_core::ValidationError;
use std::fmt::Write;
use std::time::Duration;

const MAX_SRT_BYTES: usize = 8 * 1024 * 1024;

const MAX_CUES: usize = 16_384;

const MAX_CUE_BYTES: usize = 64 * 1024;

/// Immutable caption text active in the half-open interval `[start, end)`.
///
/// Times use `Duration` precision. Text uses LF-separated nonblank lines, without
/// trailing LF, CR or control characters other than LF/tab. Markup is literal.
/// This type supplies content, not font selection, layout or a playback clock.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptionCue {
    start: Duration,
    end: Duration,
    text: String,
}

impl CaptionCue {
    /// Validates increasing times and nonblank text of at most 64 KiB in UTF-8.
    /// Blank text lines and unsupported control characters are rejected.
    pub fn new(
        start: Duration,
        end: Duration,
        text: impl Into<String>,
    ) -> Result<Self, ValidationError> {
        let text = text.into();
        if start >= end
            || text.len() > MAX_CUE_BYTES
            || text.split('\n').any(|line| line.trim().is_empty())
            || text
                .chars()
                .any(|character| character.is_control() && character != '\n' && character != '\t')
        {
            return Err(invalid(
                "Caption requires start < end and 1..=65536 bytes of nonblank LF text without unsupported controls",
            ));
        }
        Ok(Self { start, end, text })
    }

    /// Returns the inclusive cue start relative to the sampled frame's clock.
    pub fn start(&self) -> Duration {
        self.start
    }

    /// Returns the exclusive cue end relative to the sampled frame's clock.
    pub fn end(&self) -> Duration {
        self.end
    }

    /// Returns literal UTF-8 text with LF-separated lines.
    pub fn text(&self) -> &str {
        &self.text
    }
}

/// An immutable, bounded caption timeline with handwritten SRT interchange.
///
/// Build or parse outside frame callbacks. Sampling scans at most 16,384 cues,
/// preserving input order and overlaps without sorting or previous-frame state.
/// Layer construction and overlapping-label placement belong to the caller.
///
/// ```
/// use motion_content::captions::*;
/// use motion_core::prelude::*;
/// let captions = CaptionTrack::from_srt("1\n00:00:01,000 --> 00:00:03,000\nLight takes time.\n")?;
/// let time = Time::from_frame(30, fps!(30));
/// assert_eq!(
///     captions.active_at_time(time).next().unwrap().text(),
///     "Light takes time."
/// );
/// assert!(captions.to_srt()?.starts_with("1\n00:00:01,000"));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CaptionTrack {
    cues: Vec<CaptionCue>,
}

impl CaptionTrack {
    /// Keeps input order, including overlaps and unsorted times; empty input is valid.
    /// Rejects more than 16,384 cues or over 8 MiB of aggregate UTF-8 text.
    /// Cloning a track clones its text; it does not share mutable playback state.
    pub fn new(cues: impl IntoIterator<Item = CaptionCue>) -> Result<Self, ValidationError> {
        let mut track = Self::default();
        let mut bytes = 0;
        for cue in cues {
            bytes += cue.text.len();
            if track.cues.len() == MAX_CUES || bytes > MAX_SRT_BYTES {
                return Err(invalid("Caption track exceeds 16384 cues or 8 MiB of text"));
            }
            track.cues.try_reserve(1).map_err(|_| reserve_error())?;
            track.cues.push(cue);
        }
        Ok(track)
    }

    /// Returns immutable cues in their original input order.
    pub fn cues(&self) -> &[CaptionCue] {
        &self.cues
    }

    /// Samples an explicit time in the same half-open interval and input order.
    pub fn active_at(&self, time: Duration) -> impl Iterator<Item = &CaptionCue> {
        self.cues
            .iter()
            .filter(move |cue| cue.start <= time && time < cue.end)
    }

    /// Samples signed ticks exactly, without rounding through nanoseconds.
    /// Negative times contain no captions. Cue intervals remain half-open.
    pub fn active_at_time(&self, time: motion_core::Time) -> impl Iterator<Item = &CaptionCue> {
        let ticks = u128::try_from(time.ticks()).ok();
        self.cues.iter().filter(move |cue| {
            ticks.is_some_and(|ticks| {
                let position = ticks * 1_000_000_000;
                cue.start.as_nanos() * motion_core::TICKS_PER_SECOND as u128 <= position
                    && position < cue.end.as_nanos() * motion_core::TICKS_PER_SECOND as u128
            })
        })
    }

    /// Parses at most 8 MiB of UTF-8 SRT, with optional leading BOM and LF/CRLF.
    ///
    /// Requires unsigned u64 cue identifiers and `HH:MM:SS,mmm --> HH:MM:SS,mmm`;
    /// hours have at least two digits, minutes/seconds are below 60. Identifiers
    /// need not be consecutive or unique. Blank lines separate cues; surrounding
    /// blank lines and horizontal whitespace around identifiers/arrows are allowed.
    /// Multiline bodies and literal markup are preserved; timing/layout extensions
    /// are rejected. Syntax/limits return `InvalidData` with a one-based line number.
    /// Empty input is a valid empty track. No files are read during sampling.
    pub fn from_srt(source: &str) -> Result<Self, ValidationError> {
        if source.len() > MAX_SRT_BYTES {
            return Err(srt_error(1, "input exceeds 8 MiB"));
        }
        let source = source.strip_prefix('\u{feff}').unwrap_or(source);
        let mut lines = source.lines().enumerate().peekable();
        let mut cues = Vec::new();
        while let Some((number, identifier)) = lines.next() {
            if identifier.trim().is_empty() {
                continue;
            }
            if cues.len() == MAX_CUES {
                return Err(srt_error(number + 1, "more than 16384 cues"));
            }
            let identifier = identifier.trim_matches([' ', '\t']);
            if identifier.is_empty()
                || !identifier.bytes().all(|byte| byte.is_ascii_digit())
                || identifier.parse::<u64>().is_err()
            {
                return Err(srt_error(number + 1, "expected unsigned cue identifier"));
            }
            let (timing_number, timing) = lines
                .next()
                .ok_or_else(|| srt_error(number + 2, "missing timing line"))?;
            let (start, end) = timing
                .split_once("-->")
                .ok_or_else(|| srt_error(timing_number + 1, "missing --> separator"))?;
            let start = parse_timestamp(start.trim_matches([' ', '\t']))
                .ok_or_else(|| srt_error(timing_number + 1, "invalid start timestamp"))?;
            let end = parse_timestamp(end.trim_matches([' ', '\t']))
                .ok_or_else(|| srt_error(timing_number + 1, "invalid end timestamp"))?;
            if start >= end {
                return Err(srt_error(timing_number + 1, "start must precede end"));
            }
            let mut text = String::new();
            while let Some((body_number, line)) = lines.peek().copied() {
                if line.trim().is_empty() {
                    break;
                }
                let additional = line.len() + usize::from(!text.is_empty());
                if additional > MAX_CUE_BYTES - text.len() {
                    return Err(srt_error(body_number + 1, "cue body exceeds 64 KiB"));
                }
                if line
                    .chars()
                    .any(|character| character.is_control() && character != '\t')
                {
                    return Err(srt_error(
                        body_number + 1,
                        "unsupported body control character",
                    ));
                }
                text.try_reserve(additional).map_err(|_| reserve_error())?;
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(line);
                lines.next();
            }
            let cue = CaptionCue::new(start, end, text)
                .map_err(|error| srt_error(timing_number + 2, &error.to_string()))?;
            cues.try_reserve(1).map_err(|_| reserve_error())?;
            cues.push(cue);
        }
        Ok(Self { cues })
    }

    /// Serializes canonical SRT with sequential one-based identifiers and LF lines.
    /// Preserves cue order/text/times; does not sort or retain original identifiers.
    /// Rejects sub-millisecond times rather than rounding, and output over 8 MiB.
    /// No file writes, subtitle muxing or audio mixing are performed.
    pub fn to_srt(&self) -> Result<String, ValidationError> {
        let mut output = String::new();
        for (index, cue) in self.cues.iter().enumerate() {
            let start = format_timestamp(cue.start)?;
            let end = format_timestamp(cue.end)?;
            let header = format!("{}\n{start} --> {end}\n", index + 1);
            let additional = header.len() + cue.text.len() + 2;
            if additional > MAX_SRT_BYTES - output.len() {
                return Err(invalid("SRT output exceeds 8 MiB"));
            }
            output
                .try_reserve(additional)
                .map_err(|_| reserve_error())?;
            output.push_str(&header);
            output.push_str(&cue.text);
            output.push_str("\n\n");
        }
        Ok(output)
    }
}

fn parse_timestamp(value: &str) -> Option<Duration> {
    let mut parts = value.split(':');
    let hours = parts.next()?;
    let minutes = parts.next()?;
    let (seconds, millis) = parts.next()?.split_once(',')?;
    if parts.next().is_some()
        || hours.len() < 2
        || minutes.len() != 2
        || seconds.len() != 2
        || millis.len() != 3
        || [hours, minutes, seconds, millis]
            .iter()
            .any(|part| !part.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return None;
    }
    let hours = hours.parse::<u64>().ok()?;
    let minutes = minutes.parse::<u64>().ok()?;
    let seconds = seconds.parse::<u64>().ok()?;
    let millis = millis.parse::<u32>().ok()?;
    if minutes >= 60 || seconds >= 60 {
        return None;
    }
    let total = hours
        .checked_mul(3600)?
        .checked_add(minutes * 60)?
        .checked_add(seconds)?;
    Some(Duration::new(total, millis * 1_000_000))
}

fn format_timestamp(time: Duration) -> Result<String, ValidationError> {
    if !time.subsec_nanos().is_multiple_of(1_000_000) {
        return Err(invalid(
            "SRT requires exact millisecond timestamps; round explicitly before constructing cues",
        ));
    }
    let seconds = time.as_secs();
    let mut timestamp = String::new();
    write!(
        timestamp,
        "{:02}:{:02}:{:02},{:03}",
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60,
        time.subsec_millis()
    )
    .map_err(|_| invalid("Unable to format SRT timestamp"))?;
    Ok(timestamp)
}

fn invalid(message: &str) -> ValidationError {
    ValidationError::invalid("captions", "input", message)
}

fn reserve_error() -> ValidationError {
    ValidationError::limit("captions", "max_bytes", "Could not reserve caption storage")
}

fn srt_error(line: usize, message: &str) -> ValidationError {
    ValidationError::invalid("captions", "srt", format!("SRT line {line}: {message}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use motion_core::ValidationErrorKind;

    fn cue(start: u64, end: u64, text: &str) -> CaptionCue {
        CaptionCue::new(
            Duration::from_millis(start),
            Duration::from_millis(end),
            text,
        )
        .unwrap()
    }

    #[test]
    fn parses_bom_crlf_multiline_unicode_and_unsorted_overlapping_cues()
    -> Result<(), ValidationError> {
        let track = CaptionTrack::from_srt(
            "\u{feff}\r\n 42 \r\n00:00:02,000\t-->\t00:00:04,000\r\n\u{5149}\u{5e74}\r\n<i>literal</i>\r\n\r\n7\n00:00:01,000 --> 00:00:03,000\nEarlier\n",
        )?;
        assert_eq!(track.cues()[0].text(), "\u{5149}\u{5e74}\n<i>literal</i>");
        assert_eq!(track.cues()[0].start(), Duration::from_secs(2));
        assert_eq!(track.cues()[0].end(), Duration::from_secs(4));
        assert_eq!(
            track
                .active_at(Duration::from_millis(2500))
                .map(CaptionCue::text)
                .collect::<Vec<_>>(),
            ["\u{5149}\u{5e74}\n<i>literal</i>", "Earlier"]
        );
        Ok(())
    }

    #[test]
    fn canonical_srt_round_trips_order_body_and_empty_tracks() -> Result<(), ValidationError> {
        let source = "9\n00:00:01,025 --> 00:00:02,999\n First\nsecond \n\n0\n00:00:01,000 --> 00:00:04,000\nLast";
        let track = CaptionTrack::from_srt(source)?;
        let serialized = track.to_srt()?;
        assert_eq!(
            serialized,
            "1\n00:00:01,025 --> 00:00:02,999\n First\nsecond \n\n2\n00:00:01,000 --> 00:00:04,000\nLast\n\n"
        );
        assert_eq!(CaptionTrack::from_srt(&serialized)?, track);
        assert_eq!(
            CaptionTrack::from_srt("\u{feff}\r\n\n")?,
            CaptionTrack::default()
        );
        assert_eq!(CaptionTrack::default().to_srt()?, "");
        Ok(())
    }

    #[test]
    fn frame_sampling_is_half_open_and_replays_out_of_order() -> motion_core::Result {
        let track = CaptionTrack::new([cue(1000, 3000, "first"), cue(2000, 2500, "second")])?;
        for (index, expected) in [
            (60, vec!["first", "second"]),
            (0, vec![]),
            (29, vec![]),
            (30, vec!["first"]),
            (75, vec!["first"]),
            (90, vec![]),
            (60, vec!["first", "second"]),
        ] {
            assert_eq!(
                track
                    .active_at_time(motion_core::Time::from_frame(
                        i64::from(index),
                        motion_core::fps!(30)
                    ))
                    .map(CaptionCue::text)
                    .collect::<Vec<_>>(),
                expected
            );
        }
        Ok(())
    }

    #[test]
    fn fractional_frame_times_do_not_round_and_local_frames_use_local_clocks() -> motion_core::Result
    {
        let track = CaptionTrack::new([
            CaptionCue::new(
                Duration::from_nanos(333333333),
                Duration::from_nanos(333333334),
                "inside",
            )?,
            CaptionCue::new(Duration::ZERO, Duration::from_nanos(333333333), "before")?,
        ])?;
        let rate = motion_core::fps!(3);
        for frame in [
            motion_core::Time::from_frame(1, rate),
            motion_core::Time::ZERO
                + (motion_core::Time::from_frame(4, rate) - motion_core::Time::from_frame(3, rate))
                    .magnitude(),
            motion_core::Time::from_frame(1, rate),
        ] {
            assert_eq!(
                track
                    .active_at_time(frame)
                    .map(CaptionCue::text)
                    .collect::<Vec<_>>(),
                ["inside"]
            );
        }
        assert!(track.to_srt().is_err());
        Ok(())
    }

    #[test]
    fn malformed_srt_reports_invalid_data_and_a_line_number() {
        for source in [
            "one\n00:00:00,000 --> 00:00:01,000\ntext",
            "18446744073709551616\n00:00:00,000 --> 00:00:01,000\ntext",
            "1",
            "1\n",
            "1\n00:00:00,000 -> 00:00:01,000\ntext",
            "1\n0:00:00,000 --> 00:00:01,000\ntext",
            "1\n00:60:00,000 --> 01:00:01,000\ntext",
            "1\n00:00:60,000 --> 00:01:01,000\ntext",
            "1\n00:00:00.000 --> 00:00:01,000\ntext",
            "1\n00:00:00,00 --> 00:00:01,000\ntext",
            "1\n00:00:00,000 --> 00:00:01,000 X1:0\ntext",
            "1\n00:00:01,000 --> 00:00:01,000\ntext",
            "1\n00:00:02,000 --> 00:00:01,000\ntext",
            "1\n9999999999999999999:00:00,000 --> 9999999999999999999:00:01,000\ntext",
            "1\n00:00:00,000 --> 00:00:01,000\n\n",
            "1\n00:00:00,000 --> 00:00:01,000\ntext\0",
        ] {
            let error = CaptionTrack::from_srt(source).unwrap_err();
            assert_eq!(error.kind, ValidationErrorKind::InvalidValue);
            assert_eq!(error.field, "srt");
            assert!(error.message.starts_with("SRT line "));
        }
        let error = CaptionTrack::from_srt("1\ninvalid\ntext").unwrap_err();
        assert!(error.message.starts_with("SRT line 2:"));
    }

    #[test]
    fn cue_validation_rejects_empty_lines_controls_and_nonincreasing_times() {
        for text in ["", " ", "line\n\nnext", "line\n", "line\r\nnext", "line\0"] {
            assert!(CaptionCue::new(Duration::ZERO, Duration::from_secs(1), text).is_err());
        }
        assert!(CaptionCue::new(Duration::ZERO, Duration::ZERO, "text").is_err());
        assert!(CaptionCue::new(Duration::from_secs(1), Duration::ZERO, "text").is_err());
        assert!(CaptionCue::new(Duration::ZERO, Duration::from_secs(1), "line\ttext").is_ok());
    }

    #[test]
    fn limits_bound_parsing_construction_and_serialization() -> Result<(), ValidationError> {
        assert!(CaptionTrack::from_srt(&"x".repeat(MAX_SRT_BYTES + 1)).is_err());
        assert!(
            CaptionTrack::from_srt(&"1\n00:00:00,000 --> 00:00:01,000\nx\n\n".repeat(MAX_CUES + 1))
                .is_err()
        );
        assert!(CaptionTrack::new(std::iter::repeat_n(cue(0, 1, "x"), MAX_CUES + 1)).is_err());
        let large = "x".repeat(MAX_CUE_BYTES);
        assert!(
            CaptionCue::new(Duration::ZERO, Duration::from_secs(1), format!("{large}x")).is_err()
        );
        assert!(
            CaptionTrack::from_srt(&format!("1\n00:00:00,000 --> 00:00:01,000\n{large}x")).is_err()
        );
        let large = cue(0, 1, &large);
        assert!(
            CaptionTrack::new(std::iter::repeat_n(
                large.clone(),
                MAX_SRT_BYTES / MAX_CUE_BYTES + 1
            ))
            .is_err()
        );
        let track = CaptionTrack::new(std::iter::repeat_n(large, MAX_SRT_BYTES / MAX_CUE_BYTES))?;
        assert!(track.to_srt().is_err());
        Ok(())
    }

    #[test]
    fn very_large_durations_round_trip_without_timestamp_overflow() -> Result<(), ValidationError> {
        let start = Duration::new(u64::MAX - 1, 0);
        let end = Duration::new(u64::MAX, 999_000_000);
        let track = CaptionTrack::new([CaptionCue::new(start, end, "far future")?])?;
        assert_eq!(CaptionTrack::from_srt(&track.to_srt()?)?, track);
        assert_eq!(track.active_at(start).count(), 1);
        assert_eq!(track.active_at(end).count(), 0);
        Ok(())
    }
}
