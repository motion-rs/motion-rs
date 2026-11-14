# 导出系统与 FFmpeg 设计契约

本页面记录导出链路的设计契约：时间轴上的帧如何变为像素、像素如何送达编码器、支持哪些编解码器，以及产物文件如何提交或丢弃。[渲染与导出设计](rendering.md)负责覆盖率计算、缓存与预算；[系统可靠性](reliability.md)负责全局错误模型。[导出教程第 11 章](../tutorial/11-export.md)详细讲解了操作工作流（PNG 序列示例、`ExportOptions`、编码器生命周期、进度回调与踩坑点）；本文档重点阐明其背后的不变式契约。

## 流水线模型

```text
时间轴帧索引 -> Time::from_frame -> FrameRenderer::render_output_at
  -> FrameBuffer (格式符合 Encoder::input_format) -> Encoder::write -> Encoder::finish
```

[`export`](../../../crates/motion-encode/src/export.rs) 负责统一编排内置与自定义编码器（`ExportOptions::encoder`）；它在调用者主线程上运行，始终保证按帧严格保序写入。

| 阶段 | 契约与职责 |
| --- | --- |
| 准备阶段 (Preparing) | 音频完成离线准备，配置选项完成校验与规范化（`selected`），在任何实际输出文件生成前先行执行目标路径与硬件/软件编码能力检查。 |
| 渲染阶段 (Rendering) | 帧 `n` 在 `Time::from_frame(n, rate)` 时刻求值；编码器的 `input_format()` 决定其像素格式且绝不发生二次错误解释。 |
| 终结阶段 (Finalizing) | `Encoder::finish` 冲刷编码器尾部数据，随后执行原子事务提交。`ExportReport` 仅描述已成功提交的产物。 |

`ExportPhase` 的生命周期通知严格保序（`Preparing`、`Rendering`、`Finalizing`、`Completed`）；仅当 `finish` 成功返回后才会发出 `Completed`。`frame_completed` 统计的是已经交付给编码器的帧数，而非已最终提交的产物帧数。

## 纯原生 API 绑定

视频和音频编码在进程内直接通过 [`motion-ffmpeg`](../../../crates/motion-ffmpeg/src/lib.rs) 调用底层 libav* 原生 C 库函数。系统绝不启动外部 `ffmpeg` 可执行文件进程，也不提供命令行子进程降级通道；测试套件所使用的媒体信息自检工具（[`inspect.rs`](../../../crates/motion-ffmpeg/src/inspect.rs)）同样基于原生 API 构建。PNG 序列帧与 GIF 编码器由纯 Rust 实现，完全不依赖 FFmpeg。

## 帧区间与帧率换算

| 规则约束 | 源码依据 |
| --- | --- |
| `range` 为左闭右开区间，必须满足 `0 <= start < end <= duration`。 | [`render.rs`](../../../crates/motion-encode/src/export/render.rs) |
| 区间起止时间向上取整到下一个帧时刻点；不包含任何完整帧的时间区间直接被拒绝。导出报告携带规范化后的实际区间。 | 同上 |
| 输出帧率默认与时间轴帧率完全相同，唯一的例外是 `Codec::Gif`，它按自身独立的 `frame_rate` 进行时间采样。 | 同上 |
| `workers` 必须介于 `1..=64` 之间，且 `max_parallel_bytes` 必须为正数。 | 同上 |

## 编解码器矩阵

在 [`options.rs`](../../../crates/motion-encode/src/export/options.rs)、[`settings.rs`](../../../crates/motion-ffmpeg/src/settings.rs) 与 [`ffmpeg.rs`](../../../crates/motion-encode/src/encoder/ffmpeg.rs) 中实现并验证：

| `Codec` 变体 | 封装容器 / 底层库 | 输入像素格式 | 透明通道 Alpha | 核心参数与说明 |
| --- | --- | --- | --- | --- |
| `H264 { quality, preset }` | MP4 / `libx264` | `Yuv420` 8-bit | 不支持 | CRF 取值 `0..=51` |
| `H265 { quality, preset, bit_depth, params }` | MP4 / `libx265` | `Yuv420` 8 或 10-bit | 不支持 | `BitDepth::Ten` 选择 Main10（`yuv420p10le`）；可选 `params` 传递 x265 专属参数串（最多 4096 字节，严禁 NUL 字符） |
| `Vp9 { quality, alpha }` | WebM / `libvpx-vp9` | `Yuv420`；开启 `alpha` 时为直通 `SrgbRgba8` | 可选支持 | 质量参数 `0..=63`；导出 VP9 时直接拒绝时间轴音频 |
| `ProRes4444` | MOV / `prores_ks` | 直通 `SrgbRgba8` (`yuva444p10le`) | 默认支持 | 固定广播级画质，无额外可调参数 |
| `PngSequence` | 存放 PNG 帧的目录 | 编码器自行定义 | 默认直通 | 无音频轨，无视频容器元数据 |
| `Gif { frame_rate, colors }` | GIF | 编码器自行定义 | 支持索引透明 | `2..=256` 色；单帧延时必须能够放入 `1..=65535` 厘秒（分秒）范围内 |

更进一步的边界约束：编码器专属的配置仅存在于使用它们的特定枚举变体中（如 H.264/H.265 的 `preset`，H.265 的 `bit_depth` 与 `params`），`ExportOptions` 本身不携带多余字段；`preset` 预设为 `medium`、`slow` 或 `slower`；原生编码器强制要求分辨率各维度为偶数，单边不超过 16,384 像素（`EncoderCapabilities`），总帧数在 1 至 `u32::MAX` 之间，总时长不超过 86,400 秒。伴音编码为 AAC，采样率支持 44,100、48,000（默认）或 96,000 Hz。通过 `ExportOptions::codec` 选取支持 Alpha 通道的编码器、PNG 或 GIF 会自动清除默认的不透明纯黑底色；其余编码器默认将透明内容合成在黑色背景上。若链接的 FFmpeg 动态库中缺少某个编码器，在 `open` 时会直接抛出 `ErrorKind::Unsupported` 错误（由 `VideoCodec::available` 预检）。

色彩元数据规范：原生视频默认标记为 BT.709 色域基色与转换矩阵，受限范围（Limited Range）；色彩传输特性默认标记为 sRGB（IEC 61966-2-1），在设置 `gamma24` 时标记为 BT.709（[`codec.rs`](../../../crates/motion-ffmpeg/src/encoder/codec.rs)）。MP4 导出启用 `+faststart` 标记（将 moov 原子前置以便网页秒开），视频时间刻度（timescale）与章节标记采用每秒 705,600,000 ticks。容器级标签（最多 64 个）与章节信息（最多 4096 个，有序且不得相互重叠）在 [`encoder.rs`](../../../crates/motion-ffmpeg/src/encoder.rs) 中完成校验。

## 像素数据交换

[`output.rs`](../../../crates/motion-render/src/output.rs) 定义了 CPU 与 GPU 后端共享的 `PixelFormat` 格式契约：包含 `LinearRgba16Float`、`LinearRgba32Float`、`SrgbRgba8 { premultiplied }` 与 `Yuv420 { bit_depth }`（8 或 10-bit，偶数尺寸，BT.709 受限范围，10-bit 存储为小端序 `u16`）。`FrameBuffer` 拥有紧凑打包的平面，附带字节跨度（Stride）。[`yuv.rs`](../../../crates/motion-render/src/output/yuv.rs) 负责 4:2:0 分块色彩空间转换；显示变换路径在执行色彩量化时额外应用 4x4 有序抖动。不透明的 `SrgbRgba8` `FrameBuffer`（[`rgb.rs`](../../../crates/motion-render/src/output/rgb.rs)）支持 `write_ppm` (P6) 与 `write_rgb`；它们不依赖 FFmpeg 也无需遵循视频防覆盖策略，无条件覆写目标路径。

## 流式处理与多线程架构

导出流水线绝不会在内存中缓存整部短片的画面帧。原生编码器运行在独立的专用 `motion-encoder` 线程上，由容量为 2 帧的有界队列负责供给画面（[`worker.rs`](../../../crates/motion-encode/src/encoder/ffmpeg/worker.rs)）；libav* C 对象被完全限制在该线程内部，工作线程发生 panic 会自动转换为 `Error::Encoder`。

CPU 工作线程池使用容量为 `workers` 个槽位的私有 Rayon 线程池（[`parallel.rs`](../../../crates/motion-encode/src/export/parallel.rs)）。第 `i` 个槽位按步长负责渲染帧 `i, i+workers, ...`；导出线程严格按帧索引顺序依次消费结果，确保并发执行的输出顺序和比特完全等价于串行执行的结果（由测试用例 `parallel_export_matches_serial_pixels_ntsc_ranges_and_progress` 保障）。第一帧始终先在主线程串行渲染以测量单帧峰值内存；并发工作线程数受限于总帧数与 `max_parallel_bytes`。如果后续某一帧超出槽位的字节预算，仅有该 `max_bytes` 结构性失败才会触发从该帧开始无缝回退为串行继续执行；其余算法错误与 panic 均直接向上冒泡。因此场景构建回调必须保证可重复求值。

GPU 导出路径（[`gpu.rs`](../../../crates/motion-encode/src/export/gpu.rs)，需要 `gpu` 特性）在满足以下条件时生效：后端非 CPU、导出的帧数大于 1、像素格式可直接在 GPU 端打包，且确实成功初始化了 GPU 上下文。CPU 准备槽位将任务送入有界的 `FrameQueue`，其产物被保序完成并编码。

## 事务性落盘与防覆盖保证

| 输出形式 | 事务安全机制 |
| --- | --- |
| 视频文件、GIF | 在目标目录下生成临时文件 `.motion-video-*` / `.motion-gif-*`，渲染完成后使用 `persist`（允许覆盖）或 `persist_noclobber` 原子提交。 |
| PNG 序列帧 | 在临时目录中生成，随后通过 `renameat_with` 系统调用原子替换目标目录（无覆盖使用 `NOREPLACE`，覆盖使用 `EXCHANGE`；覆盖要求目标必须为真实目录）。 |
| 自定义编码器 | 必须自行维护事务边界；在遇到错误或被 Drop 时必须彻底清除残留的半成品文件。 |

未传入 `overwrite` 时，若目标文件已存在，流水线会在打开编码器或渲染第一帧画面之前立即报错 `AlreadyExists`。开启 `overwrite` 后，旧文件在最终事务提交完成前保持完全不受破坏。若已存在的目标文件同时也是当前时间轴的输入音频源，即使传入了 `overwrite` 也会被直接拒绝。任务取消、抛出异常或对象 Drop 时会自动彻底清理临时文件（[`native.rs`](../../../crates/motion-encode/src/encoder/ffmpeg/native.rs)）。

## 协作式取消与错误处理

`ProgressSink::is_cancelled` 会在任务启动前、各帧之间、等待队列阻塞时（工作线程每 10 ms 轮询一次）以及冲刷编码器时密集轮询；一旦被取消立即返回 `Error::Cancelled` 且不执行最终文件提交。渲染错误通过 `at_frame` 挂载发生错误的具体帧号；工作线程槽位或并行首帧抛出的 panic 会被捕获并转化为场景 panic 错误。编码器底层失败封装为 `Error::Encoder`，内含底层 `io::Error` 以及 FFmpeg 的原生错误码与文本消息。`Encoder::write` 严禁乱序写入帧索引、分辨率不匹配或格式错误，`finish` 会校验写入的总帧数是否完整。

## 平台要求与依赖项

| 依赖要求 | 说明细节 |
| --- | --- |
| `ffmpeg` 特性 (`motion-encode`, `motion`) | 启用原生视频/音频编码；若未开启，视频编码器在 `open` 时报错 `Unsupported`，但纯 Rust 实现的 PNG 序列与 GIF 仍可正常导出。facade 中默认启用。 |
| `gpu` 特性 | 启用 GPU 端打包与导出流水线。 |
| 构建环境依赖 | 需要系统安装 `pkg-config`、FFmpeg C 开发库（libavformat, libavcodec, libavfilter, libswscale, libswresample, libavutil）以及用于 bindgen 绑定的 libclang（[`build.rs`](../../../crates/motion-ffmpeg/build.rs)）。 |
| 运行时动态库 | 链接的动态库必须包含实际所需的编码器（如 `libx264`、`libx265`、`libvpx-vp9`、`prores_ks`、AAC）。在 Unix 平台上通过二进制内置的 rpath 查找动态库。 |

## Unsafe 代码的严格边界

全工程的所有 FFI 外部函数调用均集中在 `motion-ffmpeg` 内部（涵盖 `lib.rs`、`settings.rs`、`audio.rs`、`inspect.rs`、`encoder.rs` 及其 `codec`/`mux` 模块）。工作区配置了 `unsafe_op_in_unsafe_fn = "deny"` 以及 `clippy::undocumented_unsafe_blocks = "warn"`：每个 `unsafe` 代码块均必须附带 `// SAFETY:` 注释详述其所有权或生命周期不变式保证。`Encoder` 独占拥有其 libav 结构体指针，并在 `Drop` 时自动安全释放，且 `Drop` 过程关闭输出绝不触发未决提交。bindgen 绑定仅白名单暴露桥接所需的最小函数集；`motion-encode` 以及工作区其余所有包仅使用安全的纯 Rust 代码。

## 测试与质量验证

测试套件重点校验实际解码出的画面与音频内容，而非浅层断言 API 调用顺序：`inspect` 从生成的文件中回读流信息、准确帧数、AAC 音频与 10-bit 亮度平面；自动化测试严格断言编码格式、位深、色彩标记、目标文件未破坏以及临时文件干净清理（[`export_tests.rs`](../../../crates/motion-encode/src/encoder/ffmpeg/export_tests.rs)、[`export/tests.rs`](../../../crates/motion-encode/src/export/tests.rs)）。自定义编码器测试使用录制型 `Encoder` 严格检验帧顺序、任务取消以及能力匹配。

## 相关参考

- [导出教程](../tutorial/11-export.md)
- [渲染与导出设计](rendering.md)、[系统可靠性](reliability.md)
- [开发维护检查](../development.md)
- 源码实现：[`export.rs`](../../../crates/motion-encode/src/export.rs)、[`options.rs`](../../../crates/motion-encode/src/export/options.rs)、[`motion-ffmpeg`](../../../crates/motion-ffmpeg/src/lib.rs)
