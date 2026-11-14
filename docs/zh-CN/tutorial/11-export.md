# 11 视频与动图导出

## 本章学习目标

将同一个 Timeline 送入保序的编码流水线，按需导出 PNG 序列帧、GIF 动图或原生系统级视频文件，并正确处理目标防覆盖保护、进度回调与协作式取消。
PNG 序列帧与 GIF 导出完全不需要 `ffmpeg` 特性；原生 MP4/MOV/WebM 视频编码需要开启 `ffmpeg` 特性。

## 最小可运行示例：导出 PNG 序列帧

```rust,no_run
use motion::encode::NoProgress;
use motion::prelude::*;
use motion::render::{FrameBuffer};
use motion::extend::{Encoder, ProgressSink};

fn main() -> motion::Result {
    let timeline = Timeline::new(Size::new(320.0, 180.0), fps!(30)).track(|track| {
        track.scene(1.seconds(), |context| -> motion::Result<_> {
            Ok(Circle::new(Point::new(40.0 + context.seconds() * 240.0, 90.0), 20.0)?
                .fill(Color::rgb(20, 150, 125)))
        })
    }).build()?;
    let options = ExportOptions::new("tutorial-frames").codec(Codec::PngSequence);
    let report = export(&timeline, &options, &NoProgress)?;
    println!("{} frames -> {}", report.frames, report.path.display());
    Ok(())
}
```

导出目标是一个存放序列帧的目录，而非单张 PNG 文件。若仅需单张图片，请使用 `render_still`。
已存在的输出目录默认受到严格的防覆盖保护。切勿手动删除目录来绕过库底层的事务安全边界。

## 格式选择与 MP4 编码

`ExportOptions::new("film.mp4")` 默认采用 H.264 编码、CRF 18 画质、Medium 预设与不透明纯黑背景。它需要本地环境安装带 libx264/AAC 的 FFmpeg 开发库。单单修改文件扩展名绝不会自动改变底层编码器。

| 编码器变体 | 输出特性与核心约束 |
| --- | --- |
| `H264` / `H265` | MP4 封装；受限范围 BT.709 色彩；不支持透明视频通道 |
| `Vp9 { alpha, .. }` | WebM 封装；可选支持透明 Alpha 通道；当前暂不支持伴音音频轨 |
| `ProRes4444` | MOV 封装；原生广播级支持 Alpha 透明通道 |
| `PngSequence` | 序列帧目录；完整支持 Alpha 透明通道；无音频轨 |
| `Gif` | 2–256 色，1 位布尔透明，独立帧率；无音频；最高支持 100 fps |

编码器能力在实际打开输出前会经过严格预检。偶数分辨率、尺寸上限、色彩空间、音频支持与输入像素格式是契约性要求；扩展名不会自动对其进行隐式修正。

在开启 `ffmpeg` 特性的前提下，可直接导出视频：

```rust,no_run
use motion::encode::NoProgress;
use motion::prelude::*;

fn main() -> motion::Result {
    let timeline = Timeline::new(Size::new(320.0, 180.0), fps!(30)).track(|track| {
        track.scene(2.seconds(), |context| -> motion::Result<_> {
            Ok(Circle::new(context.center(), 20.0 + context.progress() * 30.0)?
                .fill(Color::rgb(20, 150, 125)))
        })
    }).build()?;
    export(&timeline, &ExportOptions::new("tutorial-film.mp4"), &NoProgress)?;
    Ok(())
}
```

包含完整伴音的短片导出请参阅 [audio_export 示例](../../../examples/audio_export.rs)。MP4/MOV 音频统一采用 AAC 编码；忽略音频轨并不意味着 PNG/GIF 导出了音频。

## ExportOptions 配置项速查

| 配置选项 | 作用与含义 |
| --- | --- |
| `codec` | 显式指定封装容器与视频编码格式 |
| `size` | 最终输出的物理像素尺寸，独立于逻辑设计尺寸 |
| `range` | 左闭右开的绝对时间截取区间 |
| `backend` | 渲染后端（开启 `gpu` 特性时默认为 GPU） |
| `render` | 细粒度配置资产库、多级缓存、画质模式、背景色与预算限制 |
| `workers` | 请求的并发渲染工作线程数；设为 1 保持单线程串行求值 |
| `overwrite` | 仅在最终全部渲染成功提交时，才允许覆写已有目标文件 |

时间区间自动映射至精准的帧边界。场景求值闭包始终保持绝对场景时间，而编码器帧索引与视频 PTS 时间戳则从 0 开始起算。严禁为了截取某个片段而盲目将所有闭包内的相对时间全部改乱。

## 编码器生命周期模型

`export` 默认调度内置编码器；`ExportOptions::encoder` 支持传入自定义 `Encoder` 的工厂闭包，从而保持 `export` 作为唯一的总入口。

1. 全面核验配置、分辨率、音频支持与选定时间区间。
2. 准备裁剪后的混音流，随后调用 `Encoder::open` 初始化。
3. 调用 `write` 写入从 0 开始严格递增的连续帧，绝不允许乱序交付。
4. 在所有画面帧均成功写入且未触发取消的前提下，调用 `finish` 冲刷尾部数据并执行事务落盘提交。

`input_format` 必须与 `FrameBuffer` 的内存平面、Alpha 与色彩空间严格匹配。`capabilities` 默认较为宽松；自定义编码器必须自行如实声明并校验其自身的硬件/软件约束。

内置的原生编码器在专有的后台线程中创建、运行与销毁。视频帧通过有界内存队列传递，无需每帧生成中间裸流文件。

## 防覆盖保护与事务安全性

收到成功的 `ExportReport` 才是文件已成功提交落盘的唯一标志。中途取消、对象 Drop、渲染报错、工作线程崩溃或编码异常都会立即终止任务并自动清理临时暂存文件，完整保留原有的目标文件。

自定义 `Encoder` 必须自行负责其输出事务与 Drop 清理。仅实现 `write` 并不意味着自动具备防覆盖保护。严禁将导出目标路径指向当前时间轴正在读取的音频或图片源文件。

## 进度监听与协作式取消

```rust,no_run
use motion::extend::ProgressSink;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Default)]
struct Progress {
    cancel: AtomicBool,
}

impl ProgressSink for Progress {
    fn frame_completed(&self, completed: u64, total: u64) {
        println!("{completed}/{total}");
    }

    fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Acquire)
    }
}

fn main() {
    let progress = Progress::default();
    progress.cancel.store(true, Ordering::Release);
    assert!(progress.is_cancelled());
}
```

将 `&progress` 引用传入 `export` 函数。UI 界面或另一个线程可随时将取消标志位置为 `true`。任务取消采用协作式机制，不保证能立即强行打断底层的阻塞系统调用。

`frame_completed` 反映的是已被渲染并交付给编码器的帧数，不代表文件已经完成最终写入。`ExportPhase::Completed` 以及成功返回的 `ExportReport` 才是任务成功的终点。

## CPU 并发与 GPU 任务队列

显式的 CPU 导出首先在主线程串行渲染第一帧，以此测量单帧逻辑峰值内存，随后结合剩余帧数、请求的线程数以及 `max_parallel_bytes` 动态分配合理的并发槽位。若后续某一帧超出槽位的字节预算，流水线会自动无缝从该帧起回退为单线程串行执行，因此场景回调函数必须保持纯函数性且可重现。

仅有受支持的字节准入超额才会触发这种降级重试。算法逻辑错误、计算量超限或代码 panic 依然会直接报错。帧写入与进度回调始终保持绝对保序；盲目增加工作线程数并不保证总耗时必然减少。

GPU 导出保持单一渲染会话。受支持的 GPU 端打包输出能够结合 CPU 准备槽位与有界的 GPU 任务队列，实现严格保序的完成与流式编码。总并发配额不包含全局共享缓存、编码器内部内存或物理进程 RSS。

## 常见踩坑点

- 仅修改文件扩展名而不改 `Codec` 枚举，或者误把 PNG 序列帧的输出目录当作单个文件处理。
- 误将 100% 的渲染进度条等同于文件已经完全落盘。
- 假定设置取消标志位能瞬间强杀底层的 C 库阻塞编码调用。
- 为了追求并发而在场景回调内部引入共享可变状态或副作用。
- 将工程中的素材源文件路径作为输出目标，导致输入文件在启动时被拦截报错。

## 后续章节

下一章：[12 Motion Studio](12-studio.md)。
底层契约与参考源码：[渲染与导出设计](../../zh-CN/design/rendering.md)、[导出核心实现](../../../crates/motion-encode/src/export.rs)、[音频短片导出范例](../../../examples/audio_export.rs)。
