# 10 音频处理

## 本章学习目标

学习向时间轴附加音频，深入理解数据源偏移量、播放事件、离线混音总线以及无需物理扬声器的音频分析信号。
外部音频文件解码与波形分析需要开启 `audio` 特性；原生 MP4/MOV 视频容器的 AAC 音频混流编码需要开启 `ffmpeg` 特性。纯 PCM 内存计算无需物理桌面或声卡环境。

## 最小可运行示例：波形合成、分析与视听联动

在依赖中开启 `audio` 特性。本示例无需加载外部音频文件，也不申请物理音频播放设备。

```rust,no_run
use motion::prelude::*;
use motion::extend::{AudioSource};
use motion::encode::{AudioAnalysis, AudioSignal, MixedAudio, PcmBuffer};
use std::sync::Arc;

fn main() -> motion::Result {
    let samples = (0..48_000)
        .map(|index| {
            let phase = index as f64 * 440.0 * std::f64::consts::TAU / 48_000.0;
            (math::sin(phase) * 0.2) as f32
        })
        .collect();
    let pcm = Arc::new(PcmBuffer::new(48_000, 1, samples)?);
    let loudness = AudioAnalysis::from_pcm(pcm.clone()).loudness()?;
    let sound = AudioClip::source(pcm)
        .fade_in(100.milliseconds())
        .fade_out(100.milliseconds());
    let timeline = Timeline::new(Size::new(320.0, 180.0), fps!(30))
        .audio(sound)
        .track(|track| {
            track.scene(1.seconds(), move |context| -> motion::Result<_> {
                let radius = 20.0 + context.sample(&loudness) * 80.0;
                Ok(Circle::new(context.center(), radius)?.fill(Color::rgb(20, 150, 125)))
            })
        }).build()?;
    render_still(&timeline, 500.milliseconds().after_origin(), &RenderOptions::default())?
        .save_png("tutorial-audio-reactive.png")
}
```

RMS 能量计算直接基于原始 PCM 数据；片段淡入淡出属于时间轴上的包络线。若要让画面严格跟随最终混音成品的动态变化，应当对准备好的混音流（Prepared Mix）执行分析，而非仅分析单条原始音轨。

## 数据源与播放事件

`AudioSource` 是支持多线程随机访问音频数据的扩展协议。`PcmBuffer` 是不可变的、有限的单声道/立体声 f32 PCM 内存缓冲区。克隆该结构共享底层样本数组，不共享文件读取游标。

`AudioClip` 是在时间轴上的单个播放事件（Occurrence）。数据源本身可被多个事件共享，而每个具体的摆放事件拥有各自独立的起始时间、源内偏移量、持续时长、音量、声相与淡入淡出包络。

| 操作方法 | 含义与用法 |
| --- | --- |
| `AudioClip::source` | 包装显式的内存 PCM 数据或自定义数据源 |
| `AudioClip::file` | 声明延迟加载的外部音频文件引用，创作时不执行即时解码 |
| `start` | 在时间轴全局时间线上的起始摆放时刻 |
| `offset` | 从原始数据源内部的哪个相对时刻开始读取音频 |
| `duration` | 本次播放事件在时间轴上占用的持续时间 |
| `volume` / `pan` | 静态音量增益与立体声声相定位 |
| `fade_in` / `fade_out` | 片段首尾的淡入淡出时间包络 |
| `volume_envelope` | 随时间动态变化的显式音量自动化包络线 |

`start(1.seconds())` 绝不会剪裁掉音频开头的第 1 秒内容；那是 `offset` 的职责。同一个数据源在不同时刻多次发声需要创建多个显式的 AudioClip。

## 共享旁白时钟 (Shared Narration Clock)

开启 `captions` 特性后，`CueSheet` 结合 CaptionTrack、有界正数 `std::time::Duration` 与唯一具名标记点。时间线索精确保留纳秒边界；采样时通过整数叉乘与时间轴 ticks 比对。导出交换仍要求可表达的毫秒级时间戳。

`elapsed(time, marker)` 在标记点之前或整个时间表结束时返回空；它不会在下一个标记点提前中止。`progress(time, start, end)` 钳制进度并拒绝相等或逆序的时间端点。重定时采用精确整数比例与最近纳秒，拒绝退化收缩的线索，且需要重新构建派生布局。它不隐式拉伸音频音高，也不自动更新缓存数据。

`audio_clip` 在指定线索处摆放显式源区间，并拒绝超出线索长度的区间。源区间采用 Time 衡量。系统不提供自动语音合成、强制截断、变调或自动闪避；常规 AudioClip/MixedAudio 统一提供预览与导出支撑。

## 文件音频与离线混音

本示例需要开启 `audio` 特性并在本地提供 `voice.wav` 与 `music.mp3` 音频文件：

```rust,no_run
use motion::prelude::*;
use motion::encode::{MixOptions, MixedAudio};

fn main() -> motion::Result {
    let narration = AudioClip::file("voice.wav")
        .start(1.seconds().after_origin())
        .duration(4.seconds())
        .fade_in(100.milliseconds());
    let music = AudioClip::file("music.mp3")
        .offset(2.seconds())
        .duration(6.seconds())
        .volume(0.12);
    let timeline = Timeline::new(Size::new(320.0, 180.0), fps!(30))
        .audio(narration)
        .audio(music)
        .duration(6.seconds())
        .build()?;
    let _mixed = MixedAudio::new(&timeline, MixOptions::new(48_000))?;
    Ok(())
}
```

`MixedAudio::new` 接收 `MixSource`（`&Timeline` 可自动转换传入）与 `MixOptions`（目标采样率、可选时间区间 `range` 与重采样算法）。混音器自行解析并读取各音频文件；时间轴定义本身绝不发生修改。已经提前解析好音频的调用方可通过 `MixSource::resolved(&timeline, &resolved)` 传递 `Timeline::resolve_audio` 的产物，零散片段通过 `MixSource::clips(&clips, duration)` 传入。解析过程读取文件元数据与所需时间窗口。文件源始终保持为随机访问的文件句柄，而非永久常驻的巨大静态 PCM。在执行期间请勿篡改文件。使用显式的即时 PCM 解码可以生成完全脱离文件生命周期的独立快照。

原生时间轴音频输出默认采用 48 kHz 立体声；针对 AAC 视频编码 `ExportOptions::audio_sample_rate` 同样支持 44.1 和 96 kHz。不同源采样率会自动完成高保真重采样。请使用整数 ticks 和精确采样边界保持对齐，严禁在计算音频时长前先粗暴四舍五入视频帧率。

## Master 主输出、混音总线与效果器

主输出处理属于显式可选开启项。默认配置保持原有的浮点混音；调用 `Master::safe()` 启用一个 -1 dB 的防爆音限幅器（带 5 ms 前瞻和 100 ms 释放）。响度归一化会在选定单一增益前测量 K 加权的有门限积分响度，受制于请求的重建真实峰值（True Peak）上限。

```rust,no_run
use motion::prelude::*;
use motion::audio::{MixOptions, MixedAudio, synth::{Synth, Wave}, effects::{Effect, EqKind}};
use std::sync::Arc;

fn main() -> motion::Result {
    let tone = Synth::new(Wave::Sine, 440.0, 48_000, 17)?;
    let mut music = AudioBus::new("music");
    music.gain_db = -6.0;
    music.effects.push(Arc::new(Effect::Eq { kind: EqKind::LowPass, frequency: 8_000.0, q: 0.707, gain_db: 0.0 }));
    let timeline = Timeline::new(Size::new(320.0, 180.0), fps!(30))
        .audio_bus(music)
        .audio(AudioClip::source(tone).duration(2.seconds()).bus("music").fade_in(50.milliseconds()))
        .master(Master::safe()).build()?;
    let mixed = MixedAudio::new(&timeline, MixOptions::new(48_000))?;
    let _report = mixed.loudness_report()?;
    let _levels = mixed.bus_meters()[0].level(500.milliseconds());
    Ok(())
}
```

`Effect` 涵盖 RBJ 均衡器、压缩器、噪声门、齿音消除、延迟与 Schroeder 混响。`AnimatedEq` 在片段原生时钟上每 64 帧采样一次自动化频率/Q值/增益，并平滑插值滤波系数。效果器在片段级增益/声相之前运行；总线效果器在总线增益与 Master 之前运行。显式声明的延迟/混响尾音（Tail）会自动延长推导的输出时长，而时间轴上的显式片长设定则会对其进行精准裁剪。

带内部状态的 DSP 算法从片段起点一次性烘焙计算，绝不依赖有限长度的 IIR 预热近似。不可变的已准备读取流与输出时间窗口随后能够实现位完全相同的绝对重放，包括 Master 响度标准化。克隆的准备实例共享已缓存的响度分析报告。全局有界缓存额外基于采样率与 PCM 内容哈希建立索引，命中时核验采样位以彻底排除哈希碰撞。

`PanLaw` 选择立体声平衡定律；`stereo_width` 采用 M/S 算法。`spatial` 将屏幕 2D 坐标映射为声相与反比距离衰减，宽度以听者到屏幕边缘的距离衡量，而非 HRTF。报告中的相关性指标是全片单声道兼容性的重要参考。

## 动态闪避 (Ducking)

`clip.ducked_by(&narration, Duck::new(-45.0, 12.0))` 在触发音频发声时自动压低当前片段音量；`AudioBus::ducked_by(DuckTrigger::Bus("voice".into()), duck)` 可直接对整条混音子总线执行闪避（触发总线必须在先前已声明）。触发源首先完成渲染，随后通过 5 ms 峰值保持检测器、受 `depth_db` 限制的阈值/比率增益计算器以及 dB 域平滑完成包络构建。`lookahead` 前瞻缓冲提前执行降音量操作，确保人声开口前伴奏已自然下潜。所有计算均离线完成，区间重放位完全一致。片段触发源必须为片段，总线触发源仅在总线级别生效。单个片段最多绑定 4 个触发项，嵌套场景会自动随片段平移触发时序。

## 程序化声音合成与音频剪辑

`audio::synth` 提供振荡器、带种子的白/粉/棕噪声近似、解析线性调频扫频（Chirp）、`Adsr` 包络、11 种 `Sound` 经典音效预设、节拍器以及 16 步进鼓机（掩码位 1/2/4 分别对应底鼓/军鼓/踩镲）。库本身不捆绑第三方音频文件。

引入 `motion::audio::AudioClipExt` 获取 `speed`（变速）、`reverse`（倒放）、`loop_until`（循环至指定时刻）、`crossfade`（交叉淡入淡出）与 `pitch_shift`（变调）。这些变换在片段效果器执行前先冻结裁剪后的数据源。变速采用带窗 Sinc 插值；变调与变速解耦采用有界的 WSOLA 算法，不保证共振峰不变。变速操作会等比缩放片段自带的淡入淡出，但显式声明的包络线与声相自动化保持原有时间轴。循环接缝采用等功率交叉淡入淡出。

Scene 场景仅负责画面。若需同时复用画面与声音，请构建一个包含音频的小 `Timeline` 并通过 `TrackBuilder::nest`（或 `nest_for` 显式指定时长）进行挂载：嵌套机制会在构建期一次性平移并裁剪子音频，同时完备保留子级的 Master 与总线配置。`Timeline::visual()` 仅提取纯画面，方便在 `SceneContext::embed` 中使用。

## 无物理设备的音频离线分析

`AudioAnalysis::from_pcm` 共享已解码数据；`from_source` 冻结显式时间区间。分析迹线是可反复复用的 `AudioSignal`，在画面帧回调中采样该信号既不发生文件读取也不进行实时 FFT 计算。

`loudness()` 返回线性 RMS 能量，非 LUFS，非 dB。`band(low, high)` 返回选定频段内的 RMS 能量，基于汉宁窗并进行奈奎斯特采样率检查。它绝不对原始音轨施加破坏性滤波。

用于动态图形的分析窗口在时间上向前探索；这与实时麦克风输入的因果物理延迟截然不同。反相立体声绝不会在声道求和阶段错误相互抵消能量。

`spectrum(bands, low, high)` 在单次 STFT 中计算 1..=64 个对数频段。`chroma`（十二平均律半音频级）、优势频率 `pitch`（音高）、`onset_strength`（起音强度）、`onsets`（起音节拍点）与 `beat_grid`（节拍网格）共享有界内存迹线。音高检测不是基频转录；对复杂交响乐自动测速可能会锁定高次谐波，因此 `BeatGrid::new` 允许手动指定 BPM。分析信号支持 `attack_release` 动态平滑、`log_scale` 对数压缩、分位数 `normalize` 归一化与 `threshold` 阈值截断。`AudioMarkers::write_json/read_json` 保留整数 ticks，校验格式版本并强制要求时间严格保序。

## 生产导出与 Studio 联动

时间区间的裁剪会精准保留原始播放事件的淡入淡出与包络时钟。视频编码器接收的是预先准备、裁剪妥当的完整混音流，而非在视频封装阶段重新解释零散的音频配置。

`MixOptions::resampler` 与 `ExportOptions::audio_resampler` 允许选取 `ResampleQuality::Sinc`（单侧 32 个零交叉点的 Blackman-Harris 带窗 Sinc 重采样），替代默认的高效 FFT 重采样器。交付标准采样率为 44100、48000 与 96000 Hz。Sinc 窗口在请求区间之外视作静音，因此末尾几帧的卷积核会自动平滑截断。效果器不存在冷启动预热差：从片段起点烘焙保证了任意时刻重放位完全一致。

调用 `export_audio(&timeline, &AudioExportOptions::new("mix.wav").format(AudioFormat::Wav24))` 支持在无需视频的前提下导出独立音频流：支持 16/24 位整型 PCM 或 32 位单精度浮点，单声道或立体声，8000..=192000 Hz，支持可选时间区间，针对 16 位输出提供确定性 TPDF 抖动，支持写入 `LIST INFO` 元数据标签（标题、艺术家、专辑、评论、版权、日期、流派、制作软件）。超过 4 GiB 的超大音频自动启用 RF64 文件头；导出 WAV 明确拒绝章节标记。目标文件写入采用临时文件原子提交，报告中会精确统计被整数格式截断的采样数。

Studio 桌面音频预览属于可选的物理设备消费者。静音仅影响当前扬声器监听，绝不从导出产物中剥离音轨。物理播放时钟与无头场景求值是清晰解耦的两个层级。

原生 FFmpeg 适配器将 f32 立体声 PCM 直接流式送入 AAC 编码器，无需在磁盘生成临时 WAV 中间文件；视频容器依然保持事务性原子提交。`AudioMetadata` 携带语言、EBU R128、ReplayGain 增益标签与互不重叠的 `AudioChapter` 章节标记。通过 `ExportOptions::audio_metadata` 传入；选定时间区间会自动裁切与重定基准章节。响度标签需调用方显式提供，系统绝不凭空捏造虚假标签。

`motion_studio::AudioReadout::from_mix(&mix, time)` 能够在不打开声卡的前提下读取积分 LUFS、采样/真实峰值、相位相关性与逐总线 RMS/峰值电平表，供 UI 仪表盘展示或执行导出前预警。

Studio 采用完全相同的效果器/总线/Master 处理链，提供高精度的多分辨率峰值波形，支持起音 ticks 吸附、节拍网格对齐、RMS 能量曲线与总线电平监测。分析报告严格针对 48 kHz 基础时间轴音频执行测量。

## 预算与计量上限

时间轴音频准备准入最多 256 个片段、16 条总线、每条总线 16 个效果器、每个效果器 30 秒尾音、16 层嵌套合成与 24 小时时长声明。常驻 PCM 采样上限为 3200 万个 f32 样本（128 MiB）；对于极度稀疏的超长音频，若准备阶段超出预算会直接报错拒绝而非静默截断。电平表迹线拥有独立的 16 MiB 上限；分析模块最多缓存 16 组 / 64 MiB、最多 100 万个窗口及有界 FFT 计算量。测量缓存额外保存最多 16 项 / 64 MiB 经验证的 PCM；超出部分仅在单实例内缓存。上述限制均为分配与工作量约束，并非系统进程 RSS 内存硬上限。

积分响度测量采用 400 ms 时间块、100 ms 步长、-70 LUFS 绝对门限与 -10 LU 相对门限。短于 400 ms 的音频不生成积分响度报告。真实峰值采用 4 倍过采样、32-tap 带窗 Sinc 插值估算。AAC 直接消费浮点 PCM，无需施加 16 位抖动。

## 常见踩坑点

- 时间轴上的摆放起始时间（`start`）与音频文件内部的偏移量（`offset`）是截然不同的概念。
- 严禁在每一帧的画面回调闭包内重复执行音频文件解码或运行昂贵的 FFT 频域变换。
- 严禁在执行期间修改被延迟音频源引用的磁盘文件，也严禁将正在读取的输入音频文件作为输出导出目标。
- 切勿直接将线性 RMS 能量当作分贝（dB）或广播级 LUFS 使用。
- 开启 `audio` 特性并不要求物理计算机必须连接扬声器。解码、混音与波形分析均为纯内存的无头计算。

## 后续章节

下一章：[11 视频与动图导出](11-export.md)。
完整可运行应用程序：[音频导出示例源码](../../../examples/audio_export.rs)。
设计契约与底层源码：[PCM 与混音实现](../../../crates/motion-encode/src/audio/pcm.rs)、[音频分析实现](../../../crates/motion-encode/src/audio/analysis.rs)。
