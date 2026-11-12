//! Shared benchmark settings, timing, summary statistics and report records.

use motion::prelude::*;
use motion::render::BackendKind;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    hint::black_box,
    path::{Path, PathBuf},
    process::Command,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

mod built {
    include!(concat!(env!("OUT_DIR"), "/motion_benchmark_build.rs"));
}

pub fn invalid(message: impl Into<String>) -> Error {
    std::io::Error::new(std::io::ErrorKind::InvalidInput, message.into()).into()
}

pub fn frame_timing_json(
    timings: motion::render::composition::FrameTimings,
    gpu_status: &str,
) -> Value {
    #[cfg(feature = "gpu")]
    let gpu = timings.gpu.map(|gpu| {
        let interval = |value: motion::render::gpu::GpuTimestamp| {
            use motion::render::gpu::GpuTimestampStatus as Status;
            let status = match value.status {
                Status::NotExecuted => "not_executed",
                Status::Valid => "valid",
                Status::Reversed => "reversed",
                Status::OutOfOrder => "out_of_order",
                Status::ExceedsWallTime => "exceeds_wall_time",
                Status::InvalidPeriod => "invalid_period",
                Status::Unrepresentable => "unrepresentable",
            };
            json!({
                "status": status, "ticks": value.ticks,
                "microseconds": value.duration.map(|duration| duration.as_secs_f64() * 1e6),
            })
        };
        json!({
            "composition": interval(gpu.composition),
            "output": interval(gpu.output),
            "readback_copy": interval(gpu.readback_copy),
            "coverage": {
                "flatten": interval(gpu.coverage[0]),
                "count": interval(gpu.coverage[1]),
                "prefix": interval(gpu.coverage[2]),
                "scatter": interval(gpu.coverage[3]),
                "carry": interval(gpu.coverage[4]),
                "coverage": interval(gpu.coverage[5]),
            },
            "timestamp_period_ns": gpu.timestamp_period_ns,
            "enclosing_wall_microseconds": gpu.enclosing_wall_time.as_secs_f64() * 1e6,
            "instrumentation_bytes": gpu.instrumentation_bytes,
        })
    });
    #[cfg(not(feature = "gpu"))]
    let gpu: Option<Value> = None;
    json!({
        "evaluation": timings.evaluation.as_secs_f64() * 1e6,
        "compile": timings.compile.as_secs_f64() * 1e6,
        "coverage": timings.coverage.as_secs_f64() * 1e6,
        "plan": timings.plan.as_secs_f64() * 1e6,
        "encode": timings.encode.as_secs_f64() * 1e6,
        "composition": timings.composition.as_secs_f64() * 1e6,
        "output": timings.output.as_secs_f64() * 1e6,
        "fused_output": timings.fused_output,
        "readback": timings.readback.as_secs_f64() * 1e6,
        "upload": timings.upload.as_secs_f64() * 1e6,
        "submission": timings.submission.as_secs_f64() * 1e6,
        "gpu_status": if gpu.is_some() { "available" } else { gpu_status },
        "gpu": gpu,
    })
}

pub fn gpu_timing_status(session: &motion::render::FrameRenderer) -> &'static str {
    if session.backend_kind() != BackendKind::Gpu {
        return "not_gpu";
    }
    #[cfg(feature = "gpu")]
    if let Some(context) = session.shared_gpu_context() {
        return if context.supports_timestamps() {
            "not_completed"
        } else {
            "unsupported"
        };
    }
    "unavailable"
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Settings {
    pub warmups: usize,
    pub samples: usize,
    pub iterations: usize,
}

impl Settings {
    pub fn from_env() -> motion::Result<Self> {
        let value = Self {
            warmups: number("MOTION_BENCH_WARMUPS", 2)?,
            samples: number("MOTION_BENCH_SAMPLES", 101)?,
            iterations: number("MOTION_BENCH_ITERATIONS", 1)?,
        };
        value.validate()?;
        Ok(value)
    }

    fn validate(&self) -> motion::Result {
        if !(1..=100).contains(&self.warmups)
            || !(5..=10001).contains(&self.samples)
            || !(1..=1000).contains(&self.iterations)
        {
            return Err(invalid(
                "Require warmups 1..=100, samples 5..=10001 and iterations 1..=1000",
            ));
        }
        Ok(())
    }
}

fn number(name: &str, fallback: usize) -> motion::Result<usize> {
    std::env::var(name).map_or(Ok(fallback), |value| {
        value
            .parse()
            .map_err(|_| invalid(format!("Invalid {name}")))
    })
}

pub fn size(default: (u32, u32)) -> motion::Result<PixelSize> {
    let input = std::env::var("MOTION_BENCH_SIZE")
        .unwrap_or_else(|_| format!("{}x{}", default.0, default.1));
    let (width, height) = input
        .split_once('x')
        .ok_or_else(|| invalid("Expected WIDTHxHEIGHT"))?;
    let width = width.parse::<u32>().map_err(|_| invalid("Invalid width"))?;
    let height = height
        .parse::<u32>()
        .map_err(|_| invalid("Invalid height"))?;
    if !(1..=4096).contains(&width) || !(1..=4096).contains(&height) {
        return Err(invalid("Benchmark dimensions require 1..=4096"));
    }
    Ok(PixelSize::new(width, height))
}

pub fn selected(name: &str, available: &[&str]) -> motion::Result<Vec<String>> {
    let input = std::env::var(name).unwrap_or_else(|_| available.join(","));
    let mut values = Vec::new();
    for item in input.split(',') {
        if !available.contains(&item) || values.iter().any(|value| value == item) {
            return Err(invalid(format!("Unknown or duplicate {name} item: {item}")));
        }
        values.push(item.to_owned());
    }
    Ok(values)
}

pub fn renderers(gpu: bool) -> motion::Result<Vec<BackendKind>> {
    let available = if gpu { vec!["cpu", "gpu"] } else { vec!["cpu"] };
    Ok(selected("MOTION_BENCH_BACKENDS", &available)?
        .iter()
        .map(|value| {
            if value == "gpu" {
                BackendKind::Gpu
            } else {
                BackendKind::Cpu
            }
        })
        .collect())
}

pub fn fold_checksum(state: u64, value: u64) -> u64 {
    state
        .rotate_left(7)
        .wrapping_add(value)
        .wrapping_add(0x9e37_79b9_7f4a_7c15)
}

pub fn fingerprint(bytes: &[u8]) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in bytes {
        hash = (hash ^ u64::from(*byte)).wrapping_mul(0x100_0000_01b3);
    }
    format!("{hash:016x}")
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Sample {
    pub elapsed_ns: u64,
    pub iterations: usize,
    pub checksum: String,
    pub frame: Option<u32>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Measurement {
    pub timing_kind: String,
    pub samples: Vec<Sample>,
    pub p50_us: f64,
    pub p95_us: f64,
    pub min_us: f64,
    pub max_us: f64,
    pub checksum: String,
}

#[cfg(any(motion_bench_engine, test))]
#[cfg_attr(
    not(motion_bench_engine),
    allow(
        dead_code,
        reason = "Shared source retains engine helpers for its unit tests"
    )
)]
pub fn measure(
    mut operation: impl FnMut() -> motion::Result<u64>,
    settings: &Settings,
    iterations: usize,
) -> motion::Result<Measurement> {
    settings.validate()?;
    if iterations == 0 {
        return Err(invalid("A measurement requires at least one iteration"));
    }
    for _ in 0..settings.warmups {
        black_box(operation()?);
    }
    let mut samples = Vec::with_capacity(settings.samples);
    for _ in 0..settings.samples {
        let mut outputs = Vec::with_capacity(iterations);
        let started = Instant::now();
        for _ in 0..iterations {
            outputs.push(black_box(operation()?));
        }
        let elapsed_ns =
            u64::try_from(started.elapsed().as_nanos()).map_err(|_| invalid("Timing overflow"))?;
        samples.push(Sample {
            elapsed_ns,
            iterations,
            checksum: format!("{:016x}", outputs.into_iter().fold(0, fold_checksum)),
            frame: None,
        });
    }
    summarize(samples, "batch-mean")
}

pub fn measure_output<Output>(
    mut operation: impl FnMut() -> motion::Result<Output>,
    mut consume: impl FnMut(Output) -> u64,
    settings: &Settings,
    iterations: usize,
) -> motion::Result<Measurement> {
    settings.validate()?;
    if iterations == 0 {
        return Err(invalid("A measurement requires at least one iteration"));
    }
    for _ in 0..settings.warmups {
        black_box(consume(black_box(operation()?)));
    }
    let mut samples = Vec::with_capacity(settings.samples);
    for _ in 0..settings.samples {
        let mut elapsed_ns = 0_u64;
        let mut checksum = 0;
        for _ in 0..iterations {
            let started = Instant::now();
            let output = black_box(operation()?);
            let elapsed = u64::try_from(started.elapsed().as_nanos())
                .map_err(|_| invalid("Timing overflow"))?;
            elapsed_ns = elapsed_ns
                .checked_add(elapsed)
                .ok_or_else(|| invalid("Timing overflow"))?;
            checksum = fold_checksum(checksum, black_box(consume(output)));
        }
        samples.push(Sample {
            elapsed_ns,
            iterations,
            checksum: format!("{checksum:016x}"),
            frame: None,
        });
    }
    summarize(
        samples,
        if iterations == 1 {
            "single-operation"
        } else {
            "mean-of-individual-operations"
        },
    )
}

#[cfg(any(motion_bench_engine, test))]
#[cfg_attr(
    not(motion_bench_engine),
    allow(
        dead_code,
        reason = "Shared source retains engine helpers for its unit tests"
    )
)]
pub fn measure_prepared<Input, Output>(
    mut prepare: impl FnMut() -> motion::Result<Input>,
    mut operation: impl FnMut(&Input) -> motion::Result<Output>,
    mut consume: impl FnMut(Output) -> u64,
    settings: &Settings,
) -> motion::Result<Measurement> {
    settings.validate()?;
    for _ in 0..settings.warmups {
        let input = prepare()?;
        black_box(consume(black_box(operation(&input)?)));
    }
    let mut samples = Vec::with_capacity(settings.samples);
    for _ in 0..settings.samples {
        let input = prepare()?;
        let started = Instant::now();
        let output = black_box(operation(&input)?);
        let elapsed_ns =
            u64::try_from(started.elapsed().as_nanos()).map_err(|_| invalid("Timing overflow"))?;
        samples.push(Sample {
            elapsed_ns,
            iterations: 1,
            checksum: format!("{:016x}", consume(output)),
            frame: None,
        });
    }
    summarize(samples, "fresh-input-operation")
}

pub fn frame_order(total: u32, shuffled: bool) -> Vec<u32> {
    let mut frames = (0..total).collect::<Vec<_>>();
    if shuffled {
        let mut random = Random::from_key(0x5065_7266_u64);
        for index in (1..frames.len()).rev() {
            let other = (random.next_float() * (index + 1) as f64) as usize;
            frames.swap(index, other.min(index));
        }
    }
    frames
}

pub fn sequence_length(total: u32) -> motion::Result<u32> {
    let frames = number("MOTION_BENCH_SEQUENCE_FRAMES", total as usize)?;
    if frames < 2 || frames > total as usize {
        return Err(invalid("Sequence length must be 2..=timeline frame count"));
    }
    Ok(frames as u32)
}

pub fn measure_frames<Output>(
    frames: &[u32],
    mut operation: impl FnMut(u32) -> motion::Result<Output>,
    mut consume: impl FnMut(Output) -> u64,
    settings: &Settings,
) -> motion::Result<Measurement> {
    settings.validate()?;
    let first = *frames
        .first()
        .ok_or_else(|| invalid("A frame sequence cannot be empty"))?;
    for _ in 0..settings.warmups {
        black_box(consume(black_box(operation(first)?)));
    }
    let mut samples = Vec::with_capacity(frames.len());
    for &frame in frames {
        let started = Instant::now();
        let output = black_box(operation(frame)?);
        let elapsed_ns =
            u64::try_from(started.elapsed().as_nanos()).map_err(|_| invalid("Timing overflow"))?;
        samples.push(Sample {
            elapsed_ns,
            iterations: 1,
            checksum: format!("{:016x}", consume(output)),
            frame: Some(frame),
        });
    }
    summarize(samples, "frame-sequence")
}

pub fn summarize(samples: Vec<Sample>, timing_kind: &str) -> motion::Result<Measurement> {
    let mut values = samples
        .iter()
        .map(|sample| sample.elapsed_ns as f64 / sample.iterations as f64 / 1000.0)
        .collect::<Vec<_>>();
    values.sort_by(f64::total_cmp);
    let checksum = format!(
        "{:016x}",
        samples.iter().fold(0, |state, sample| fold_checksum(
            state,
            u64::from_str_radix(&sample.checksum, 16).expect("generated checksum")
        ))
    );
    Ok(Measurement {
        timing_kind: timing_kind.into(),
        samples,
        p50_us: percentile(&values, 50),
        p95_us: percentile(&values, 95),
        min_us: values[0],
        max_us: values[values.len() - 1],
        checksum,
    })
}

fn percentile(values: &[f64], percent: usize) -> f64 {
    values[(values.len() * percent).div_ceil(100).saturating_sub(1)]
}

#[derive(Serialize, Deserialize)]
pub struct Record {
    pub name: String,
    pub measurement: Measurement,
    pub metrics: Value,
}

#[derive(Serialize, Deserialize)]
pub struct Report {
    pub schema: String,
    pub suite: String,
    pub label: String,
    pub settings: Settings,
    pub configuration: Value,
    pub environment: Value,
    pub provenance: Value,
    pub execution: Value,
    pub records: Vec<Record>,
    pub references: BTreeMap<String, String>,
    #[serde(skip)]
    pub directory: PathBuf,
    #[serde(skip)]
    baseline: Option<Box<Report>>,
}

fn create_output(path: &Path) -> motion::Result {
    std::fs::create_dir_all(path)?;
    if std::fs::read_dir(path)?.next().is_some() {
        return Err(invalid(
            "Output must be empty; existing benchmark results are never overwritten",
        ));
    }
    Ok(())
}

fn workspace_path(root: &Path, configured: &Path) -> PathBuf {
    if configured.is_absolute() {
        configured.to_owned()
    } else {
        root.join(configured)
    }
}

impl Report {
    pub fn new(suite: &str, settings: Settings, configuration: Value) -> motion::Result<Self> {
        if cfg!(debug_assertions) {
            return Err(invalid(
                "Use cargo bench / optimized builds for measurements",
            ));
        }
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
        let root = if manifest.join("../../crates/motion").is_dir() {
            manifest.join("../..")
        } else {
            manifest.join("..")
        }
        .canonicalize()?;
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| invalid("Invalid system clock"))?
            .as_nanos();
        let directory = std::env::var_os("MOTION_BENCH_OUTPUT")
            .map(|value| workspace_path(&root, Path::new(&value)))
            .unwrap_or_else(|| root.join(format!("target/performance/{suite}-{stamp}")));
        let environment = json!({
            "os": std::env::consts::OS, "architecture": std::env::consts::ARCH,
            "logical_cpus": std::thread::available_parallelism()?.get(),
            "rustc": built::RUSTC,
            "build":{"target":built::TARGET,"profile":built::PROFILE,"optimization":built::OPT_LEVEL,"debug":built::DEBUG,"flags":built::FLAGS,"features":built::FEATURES},
            "platform":platform_identity(),
            "optimized": !cfg!(debug_assertions),
            "machine": std::env::var("MOTION_BENCH_MACHINE").unwrap_or_else(|_| "unspecified".into()),
        });
        let baseline = if let Some(path) = std::env::var_os("MOTION_BENCH_COMPARE") {
            let path = workspace_path(&root, Path::new(&path));
            let mut value: Report =
                serde_json::from_slice(&std::fs::read(path.join("report.json"))?)
                    .map_err(|error| invalid(error.to_string()))?;
            if value.schema != "motion-bench-v3"
                || value.suite != suite
                || value.configuration != configuration
                || value.settings != settings
                || value.environment != environment
            {
                return Err(invalid(
                    "Baseline schema, workload, sampling or host/compiler configuration differs",
                ));
            }
            value.directory = path.canonicalize()?;
            Some(Box::new(value))
        } else {
            None
        };
        create_output(&directory)?;
        let files = built::SOURCE_FILES
            .iter()
            .copied()
            .collect::<BTreeMap<_, _>>();
        let executable = std::env::current_exe()?;
        std::fs::copy(&executable, directory.join("benchmark-executable"))?;
        let provenance = json!({
            "fingerprint_algorithm": "fnv1a64-noncryptographic", "files": files,
            "source_capture":"compile-time",
            "executable": "benchmark-executable",
            "executable_fingerprint": fingerprint(&std::fs::read(executable)?),
        });
        Ok(Self {
            schema: "motion-bench-v3".into(),
            suite: suite.into(),
            label: std::env::var("MOTION_BENCH_LABEL").unwrap_or_else(|_| "baseline".into()),
            settings,
            configuration,
            environment,
            provenance,
            execution: json!({"started_unix_ns":stamp.to_string(),"process_id":std::process::id()}),
            records: Vec::new(),
            references: BTreeMap::new(),
            directory,
            baseline,
        })
    }

    pub fn reference(&mut self, name: &str, bytes: &[u8]) -> motion::Result {
        if name.contains('/') || name.contains("..") || self.references.contains_key(name) {
            return Err(invalid("Reference names must be unique simple filenames"));
        }
        if let Some(baseline) = &self.baseline
            && (!baseline.references.contains_key(name)
                || std::fs::read(baseline.directory.join(name))? != bytes)
        {
            return Err(invalid(format!("Reference pixels differ: {name}")));
        }
        std::fs::write(self.directory.join(name), bytes)?;
        self.references.insert(name.to_owned(), fingerprint(bytes));
        Ok(())
    }

    pub fn record(
        &mut self,
        name: impl Into<String>,
        measurement: Measurement,
        metrics: Value,
    ) -> motion::Result {
        let name = name.into();
        if self.records.iter().any(|record| record.name == name) {
            return Err(invalid("Duplicate measurement"));
        }
        if let Some(baseline) = &self.baseline {
            let previous = baseline
                .records
                .iter()
                .find(|record| record.name == name)
                .ok_or_else(|| invalid(format!("Missing baseline case: {name}")))?;
            if previous.measurement.checksum != measurement.checksum
                || previous.metrics.get("adapter") != metrics.get("adapter")
            {
                return Err(invalid(format!("Checksum or adapter differs: {name}")));
            }
        }
        println!(
            "{name}: p50={:.3} us p95={:.3} us",
            measurement.p50_us, measurement.p95_us
        );
        self.records.push(Record {
            name,
            measurement,
            metrics,
        });
        Ok(())
    }

    pub fn finish(mut self) -> motion::Result<PathBuf> {
        if let Some(baseline) = &self.baseline
            && (baseline.records.len() != self.records.len()
                || baseline.references.keys().ne(self.references.keys()))
        {
            return Err(invalid("Baseline case/reference sets differ"));
        }
        let mut rows = String::from("case\tp50_us\tp95_us\tmin_us\tmax_us\tchecksum\n");
        let mut samples = String::from("case\tsample\titerations\telapsed_ns\tchecksum\n");
        for record in &self.records {
            let value = &record.measurement;
            rows.push_str(&format!(
                "{}\t{}\t{}\t{}\t{}\t{}\n",
                record.name, value.p50_us, value.p95_us, value.min_us, value.max_us, value.checksum
            ));
            for (index, sample) in value.samples.iter().enumerate() {
                samples.push_str(&format!(
                    "{}\t{}\t{}\t{}\t{}\n",
                    record.name, index, sample.iterations, sample.elapsed_ns, sample.checksum
                ));
            }
        }
        std::fs::write(self.directory.join("timings.tsv"), rows)?;
        std::fs::write(self.directory.join("samples.tsv"), samples)?;
        self.execution["finished_unix_ns"] = json!(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| invalid("Invalid system clock"))?
                .as_nanos()
                .to_string()
        );
        std::fs::write(
            self.directory.join("report.json"),
            serde_json::to_vec_pretty(&self).map_err(|error| invalid(error.to_string()))?,
        )?;
        println!("Raw report: {}", self.directory.display());
        Ok(self.directory)
    }
}

fn platform_identity() -> Value {
    let query = |program: &str, args: &[&str]| {
        Command::new(program)
            .args(args)
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
    };
    json!({
        "kernel":query("uname",&["-srm"]),
        "os_build":if cfg!(target_os="macos") { query("sw_vers",&["-buildVersion"]) } else { query("uname",&["-v"]) },
        "cpu":if cfg!(target_os="macos") { query("sysctl",&["-n","machdep.cpu.brand_string"]) } else { std::env::var("PROCESSOR_IDENTIFIER").ok() },
        "physical_memory":if cfg!(target_os="macos") { query("sysctl",&["-n","hw.memsize"]) } else { None },
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn samples_keep_even_checksums_and_nearest_rank_quantiles() -> motion::Result {
        use super::*;
        let settings = Settings {
            warmups: 1,
            samples: 5,
            iterations: 4,
        };
        let first = measure(|| Ok(7), &settings, 4)?;
        assert_eq!(first.timing_kind, "batch-mean");
        let mut consumed = 0;
        let output = measure_output(
            || Ok(vec![7]),
            |value| {
                consumed += 1;
                value[0]
            },
            &settings,
            1,
        )?;
        assert_eq!(output.timing_kind, "single-operation");
        assert_eq!(consumed, settings.warmups + settings.samples);
        assert_ne!(first.checksum, "0000000000000000");
        assert_eq!(first.checksum, measure(|| Ok(7), &settings, 4)?.checksum);
        assert_eq!(first.samples.len(), 5);
        assert_eq!(percentile(&[1.0, 2.0, 3.0, 4.0, 5.0], 50), 3.0);
        assert_eq!(percentile(&[1.0, 2.0, 3.0, 4.0, 5.0], 95), 5.0);
        assert!(measure(|| Ok(7), &settings, 0).is_err());
        assert!(
            Settings {
                samples: 0,
                ..settings
            }
            .validate()
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn output_is_no_clobber_and_checksums_keep_order() -> motion::Result {
        use super::*;
        assert!(
            built::SOURCE_FILES
                .iter()
                .any(|(path, _)| *path == "crates/motion-bench/benches/support.rs")
        );
        assert!(!built::RUSTC.is_empty());
        assert_eq!(
            workspace_path(Path::new("workspace"), Path::new("target/results")),
            Path::new("workspace/target/results")
        );
        let directory = tempfile::tempdir()?;
        assert_eq!(
            workspace_path(Path::new("unused"), directory.path()),
            directory.path()
        );
        create_output(directory.path())?;
        std::fs::write(directory.path().join("report.json"), b"baseline")?;
        assert!(create_output(directory.path()).is_err());
        assert_eq!(
            std::fs::read(directory.path().join("report.json"))?,
            b"baseline"
        );
        assert_ne!(
            fold_checksum(fold_checksum(0, 1), 2),
            fold_checksum(fold_checksum(0, 2), 1)
        );
        Ok(())
    }

    #[test]
    fn fresh_inputs_and_frame_schedules_are_explicit() -> motion::Result {
        use super::*;
        let settings = Settings {
            warmups: 1,
            samples: 5,
            iterations: 1,
        };
        let prepared = std::cell::Cell::new(0);
        let measured = measure_prepared(
            || {
                prepared.set(prepared.get() + 1);
                Ok(std::cell::Cell::new(false))
            },
            |value| {
                assert!(!value.replace(true));
                Ok(7)
            },
            |value| value,
            &settings,
        )?;
        assert_eq!(prepared.get(), 6);
        assert_eq!(measured.timing_kind, "fresh-input-operation");
        let first = frame_order(12, true);
        assert_eq!(first, frame_order(12, true));
        assert_ne!(first, frame_order(12, false));
        let mut sorted = first.clone();
        sorted.sort();
        assert_eq!(sorted, frame_order(12, false));
        let frames = measure_frames(&first, Ok, u64::from, &settings)?;
        assert_eq!(
            frames
                .samples
                .iter()
                .map(|sample| sample.frame.unwrap())
                .collect::<Vec<_>>(),
            first
        );
        assert!(frames.samples.iter().all(|sample| sample.iterations == 1));
        Ok(())
    }
}
