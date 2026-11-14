# 音频系统设计契约

音频系统被严格解耦为纯声明式的创作描述与离线的、确定性的准备流水线。这两层均不需要任何物理音频播放硬件。实时播放功能仅存在于桌面端 Studio 中，作为独立的消费者运行。具体类型签名请查阅 Rustdoc；本页面记录所有权归属、数值配额限制、处理顺序与故障容错行为。资产级别的上下文见[内容系统契约](content.md#创作工具与音频处理)，音视频容器封装与事务规则见[渲染与导出](rendering.md#视频与音频)。

## 所有权分层

| 层级 | 所属 Crate | 负责拥有的类型与概念 |
| --- | --- | --- |
| 描述层 | [`motion-core`](../../../crates/motion-core/src/audio.rs) | `AudioSource`、`AudioClip`、`AudioBus`、`Master`、`Duck`/`Ducking`、`PanLaw`、`AudioEffect`、`AudioMetadata`；仅负责结构描述与校验 |
| 准备层 | [`motion-encode`](../../../crates/motion-encode/src/audio.rs) | `PcmBuffer`、`MixedAudio`、`BusMeter`、重采样、内置效果器、Master 处理、流式分析、波形合成、WAV 导出 |
| 预览层 | [`motion-studio`](../../../crates/motion-studio/src/audio/preview.rs) | `cpal` 硬件播放、无锁环形缓冲区、读数统计模型 |

- `AudioClip` 是在共享的 `AudioSource` 之上的不可变播放事件。起始时间、源内偏移量、裁切区间、音量包络线与淡入淡出各自拥有独立的时钟；声相（Pan）动画在事件局部时钟上采样。
- `AudioSource` 是纯函数式的、支持多线程并发访问、支持随机寻址交错存储的 f32 音频数据源。重复读取绝不依赖先前的读取状态；短读与 EOF 结尾以静音填充剩余缓冲区；返回的采样点必须为有限数值。混音器原生支持单声道与双声道立体声。
- 外部音频文件与嵌套 Timeline 数据源为延迟解析。`Timeline::resolve_audio` 输出 `ResolvedAudio`（包含音频片段及其所需的输出时长）而不改变 Timeline 自身。`MixedAudio::new` 仅在执行边界（导出、混音、`fit_timeline`）处将其解析为真实数据源，绝不在场景求值期间触发；持有 `ResolvedAudio` 的调用方可通过 `MixSource::resolved` 传入。音频文件按时间窗口延迟解码，执行期间其磁盘内容严禁变动；显式指定的 `PcmBuffer` 内存数据被直接冻结。Timeline 嵌套层级上限为 16 层。

## 配额与阈值限制

| 限制项 | 允许阈值 | 校验生效位置 |
| --- | --- | --- |
| 音频源采样率 / 声道数 | 1..=192000 Hz，单声道或立体声 | `AudioClip::resolved_duration` |
| 单次准备混音 / 文件导出 / Studio 列表中的片段上限 | 256 (`MAX_CLIPS`) | `MixedAudio::new`，Studio 音频设置 |
| 总线数量 / 单链效果器上限 / 单片段闪避上限 | 16 / 16 / 4 | `Timeline` 校验，`AudioClip` |
| 效果器尾音时长（Tail） | 0..=30 秒；追加至片段时长之后 | `AudioClip::resolved_duration` |
| 声明的音频时长跨度 | 最大 24 小时 (`MAX_SECONDS = 86_400`) | 片段与混音校验 |
| 音量增益 / 声相 / 立体声声场宽度 | 0..=16 / -1..=1 / 0..=2 | `AudioClip::resolved_duration` |
| 内存常驻 PCM 采样上限 | 32 Mi 个 f32 采样点 = 128 MiB (`MAX_SAMPLES`) | `PcmBuffer`、混音、总线子混音、快照、重采样暂存 |
| 编码音频文件输入体积 | 64 MiB (`MAX_ENCODED_BYTES`) | `PcmBuffer::from_bytes`/`open` |
| 总线电平表迹线存储 | 16 MiB | `process_buses` |
| 元数据限制 | 64 个标签，4096 个章节，4096 字节文本 | `AudioMetadata::validate` |
| 最终混音采样率 | 1..=192000 Hz；导出 AAC 视频固定要求 44.1/48/96 kHz | `MixedAudio`，`NativeOptions::audio_mix` |

上述均为硬性的内存分配与计算工作量上限，而非宽泛的系统进程内存预估。超额的请求会被直接报错拒绝，而非隐式截断。

## 混音准备顺序

针对每个音频片段（在 `MixedAudio::new` 中）：读取数据源时间窗口 → 在**源采样率**下运行效果器链 → 重采样至混音目标采样率 → 应用静态音量、包络线与淡入淡出 → 调整立体声宽度与声相定位 → 执行片段级闪避（Ducking） → 存入 `PreparedClip`。

随后，针对全混音流统一执行一次：

1. 总线处理（`process_buses`），严格按照总线声明顺序执行。每个总线将其归属片段求和、运行总线效果器、应用静态增益，随后执行总线级闪避。未指定总线的片段直接进入主输出。引用未声明总线的片段会导致校验失败。
2. 总线电平表在效果器、增益与闪避之后，记录 50 ms RMS 能量与峰值采样迹线。
3. 主输出链路（`MixedAudio::apply_master`）：应用静态主增益 → 可选的广播级响度标准化 → 可选的前瞻限幅器（Lookahead Limiter） → 在阈值下方执行重建真实峰值（True Peak）修正。`Master::default()` 保持原样不作修改；`Master::safe()` 启用一个 -1 dB 的防爆音限幅器而不执行响度归一化。

闪避触发源（Trigger）在目标音频处理之前必须完全计算完毕。总线触发源的声明顺序必须早于被闪避的总线；片段触发源在自身增益与声相下独立渲染，其自身的闪避设置会被忽略。包络检测器、启动时间（Attack）、释放时间（Release）与前瞻缓冲在完整的触发信号上离线执行。

声相定律：默认采用 `PanLaw::ConstantPower`（等功率定律）；它与 `Minus4_5dB` 在声场中央归一化，而 `Linear` 仅衰减相反声道；单声道源始终采用等功率定律定位。

## 效果器：状态管理、尾音与随机寻址

`AudioEffect::process` 接收完整的交错音频缓冲区，初始状态一律清零。滤波器与动态处理器的内部状态仅在准备流水线运行期间短暂存在；混音器将每个音频事件从其起点一次性处理完毕并持久化缓存结果，因此后续的寻址播放绝不会遇到 IIR 滤波器的冷启动预热阶段。内置的 `Effect` 包含均衡器（EQ）、压缩器、噪声门、齿音消除器（De-esser）、延迟（Delay）与混响（Reverb）；`AnimatedEq` 在音频片段的原生时钟上对自动化参数进行连续采样。延迟与混响声明显式的 `tail` 尾音截断衰减；`tail()` 会向事件追加相应时长的静音输入。自定义效果器必须保持确定性，并在无需处理实际 PCM 数据的情况下实现 `validate` 校验。

由此产生的重要推论：仅当音频流**完全不包含**任何效果器、闪避、总线或主输出设置时，带有 `MixOptions::range` 的 `MixedAudio::new` 才会直接拉取局部时间窗口；否则流水线会完整准备整条时间轴后再做时间截取，确保效果器和响度归一化的内部时间线绝不会在时间窗口边缘发生截断重置。`MixedAudio::crop` 同样基于已准备的产物执行时间裁剪。

## 确定性重放与定位一致性

- 已准备音频读取（`MixedAudio::read`）以 f64 双精度浮点在固定顺序下累加已准备好的片段，绝不触碰原始数据源或重采样器状态；读取结果完全不依赖查询的先后次序。
- 数据源读取、效果器运算、闪避与主链路处理均严格作为时间轴与配置选项的纯函数。用户自定义的数据源与包络线被视为受信任的纯回调函数；发生 panic 时直接向上冒泡。
- 重采样器支持显式选择：`ResampleQuality::Fast`（基于 FFT，默认选项，需要 `audio` 特性）或 `Sinc`（带窗 Sinc 插值，单侧 32 个零交叉点）。
- 波形合成器（`Synth`）直接根据帧索引解析计算相位；噪声生成器为帧号与随机种子的散列值，因此在固定种子下 Seek 定位重放能够生成位完全相同的采样数据。时间与帧的转换采用纯整数算术。正弦波发生器直接读取固定 4096 段的带线性插值的查找表（误差低于 `3e-7`，约为 -130 dB），而非针对每个样本调用昂贵的 `sin` 函数。
- 音频分析信号为预先计算的冻结迹线，因此在视频帧回调函数内部采样音频信号绝不会产生任何文件 I/O 或实时 FFT 运算开销。

## 音频分析信号 (Audio Signals)

`AudioAnalysis`（需要 `audio` 特性）封装一份不可变的 `PcmBuffer`，或通过 `from_source` 冻结任意纯数据源的有界区间。它输出 `AudioSignal`（无状态的有限动画 `FiniteAnimation<f64>`，线性插值，超出时长范围返回 0.0）：包含 `loudness`（线性 RMS 能量，非 LUFS）、`band` 频段、`pitch` 音高与 `onset_strength` 起音强度；`AudioSpectrum`（包含 `spectrum` 频谱与 `chroma` 音级）；`AudioMarkers`（节奏触发点，支持 JSON 导入导出）；以及 `BeatGrid` 节拍对齐网格。分析迹线基于正向汉宁窗计算，按 PCM 实例进行内存缓存（上限 16 条迹线 / 64 MiB），时间采样点上限为 100 万。分析原始音频源并不等同于分析最终混音结果；若画面需要跟随最终成品的节奏律动，应当对准备完毕的混音流（Prepared Mix）执行分析。`WaveformPyramid` 负责构建多级峰值包络线（最底层最多 100 万个采样桶）。

## 导出流水线

- 视频导出：`export` 解析音频，以 `MixSource::resolved` 构建 `MixedAudio::new`，并在配置的 `ExportOptions.audio_sample_rate` 采样率下针对目标时间区间生成 `MixOptions::range`，在编码写入第一帧视频画面前将其交付给 `Encoder::open`。
- 独立音频导出：`export_audio` 将音频导出为标准 WAV 文件（小文件使用标准 RIFF，大文件自动采用 RF64 扩展），输出过程写入同目录下的临时文件过渡。若未显式指定 `overwrite`，已存在的文件不会被破坏；导出失败不会遗留半成品残缺文件；导出目标路径不得作为输入源。支持采样率 8000..=192000 Hz；导出 WAV 时直接拒绝包含章节元数据。
- 响度报告（`MixedAudio::loudness_report`）输出带门限的 400 ms 积分 LUFS 响度以及重建的真实峰值（True Peak）；时长短于一个测量块的音频不计算积分响度。

## Studio 预览与生产导出的差异

Studio 通过专有的后台工作线程、`cpal` 库与无锁环形缓冲区播放音频；它绝非导出流水线的一环。Studio 固定要求 48 kHz 双声道 f32 输出，以 512 帧音频块为单位处理；对于带有效果器的复杂时间轴会预渲染整条时间轴，而对于极简音频则仅准备 8192 帧的短期时间窗口。通过 `stop` 可随时终止播放请求；缓冲区欠载、声卡断开与混音异常会显式体现为 `AudioStatus::Failed` 状态，绝不干扰正式导出。`AudioReadout` 无需打开物理声卡，即可从已准备的混音流中读取积分 LUFS、采样/真实峰值、相位相关性与总线电平表。Studio 加载音频草稿时不会预先打开磁盘媒体源。详见 [Studio 用户指南](../studio.md)。

## Cargo 特性与错误分类

| Cargo 特性 | 具备的能力与影响 |
| --- | --- |
| `audio` (`motion-encode` 默认启用) | 音频解码 (WAV/MP3/AAC/FLAC/Vorbis)、FFT 高质量重采样、流式分析信号 |
| `ffmpeg` (隐式包含 `audio`) | 原生硬件/系统级 AAC 编码并封装混流至 MP4/MOV 视频中 |
| 两者皆未开启 | 显式 PCM 数据以及同采样率的简单混音依然可用；加载音频文件与跨采样率转换会直接报错 |

所有错误统一归纳为 `motion_core::Error`：非法参数返回带有字段标签的验证错误（`audio.<field>`）；无法读取或内容发生变化的文件返回资产错误；从数据源、增益或 Master 计算出的非有限浮点数直接拒绝存储。音频准备阶段产生的错误会在视频画面编码启动前立即抛出。

## 教程覆盖

[教程第 10 章](../tutorial/10-audio.md)详细讲解了音频创作：数据源引入、事件摆放、旁白时钟对齐、混音总线、动态闪避、波形合成、基于音频分析驱动视觉画面以及常见踩坑指南。本文档则负责支撑其背后的底层契约：系统分层、配额上限、处理时序与确定性重放。

## 相关参考

- [内容系统](content.md#创作工具与音频处理) · [渲染与导出](rendering.md#视频与音频) · [系统可靠性](reliability.md)
- [Studio 用户指南](../studio.md) · [`audio_export` 示例源码](../../../examples/audio_export.rs)
- 源码实现：[`mix.rs`](../../../crates/motion-encode/src/audio/pcm/mix.rs) · [`master.rs`](../../../crates/motion-encode/src/audio/master.rs) · [`effects.rs`](../../../crates/motion-encode/src/audio/effects.rs)
