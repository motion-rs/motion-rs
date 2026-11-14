# 01 快速上手

## 本章学习目标

搭建最小开发环境，生成您的第一张 PNG 静帧，并复用完全相同的场景生成视频、在 Gallery 展厅中自省以及在 Studio 桌面工作台中进行交互式调试。
首个静帧不需要任何可选特性。导出 MP4 视频需要 `ffmpeg` 特性；运行桌面工作台需要 `studio` 特性。

## 环境配置与依赖声明

请使用支持 Rust edition 2024 的编译工具链。在独立的 Cargo 应用程序配置中引用本工作区的 facade crate：

```toml
[package]
name = "motion-tutorial"
version = "0.1.0"
edition = "2024"

[dependencies]
motion-rs = { path = "/绝对路径/至/motion-rs/crates/motion", default-features = false }
```

请将 `path` 替换为您本地的代码克隆绝对路径（指向 `crates/motion` 目录，而非工作区根目录）。
后续章节将根据需求按需引入 `features = ["text"]` 等依赖特性。

也可以直接在 motion-rs 仓库根目录下运行内置的最小示例：

```sh
cargo run --no-default-features --example motion -- list
cargo run --no-default-features --example motion -- still motion --time 1 --output tutorial-first.png
```

## 生成您的第一张静帧

以下是一个完整的独立应用程序：

```rust,no_run
use motion::prelude::*;

fn main() -> motion::Result {
    let timeline = Timeline::new(Size::new(640.0, 360.0), fps!(30)).track(|track| {
        track.scene(3.seconds(), |context| -> motion::Result<_> {
            let horizontal = context.sample(&Tween::new(80.0, 560.0, 3.seconds()));
            Ok((
                Rect::new(0.0, 0.0, 640.0, 360.0).fill(Color::rgb(245, 248, 246)),
                Circle::new(Point::new(horizontal, 180.0), 32.0)?
                    .fill(Color::rgb(20, 160, 130)),
            ))
        })
    }).build()?;
    let bitmap = render_still(
        &timeline,
        1500.milliseconds().after_origin(),
        &RenderOptions::default(),
    )?;
    bitmap.save_png("tutorial-first.png")
}
```

背景为一个普通的矩形节点。若不显式绘制背景，输出的图片将保留透明区域。
`1500.milliseconds()` 代表时间轴上的具体时间点，而非整数帧号。在 30 fps 下它恰好落在第 45 帧的时间边界上。

## 通过 CLI 命令行复用相同场景

通过 `Project` 注册 Timeline，无需自行编写命令行参数解析或回放控制循环：

```rust,no_run
use motion::prelude::*;

fn main() -> std::process::ExitCode {
    Project::new("tutorial")
        .timeline("intro", || {
            Timeline::new(Size::new(640.0, 360.0), fps!(30)).track(|track| {
                track.scene(3.seconds(), |context| -> motion::Result<_> {
                    let horizontal =
                        context.sample(&Tween::new(80.0, 560.0, 3.seconds()));
                    Ok(Circle::new(Point::new(horizontal, 180.0), 32.0)?
                        .fill(Color::rgb(20, 160, 130)))
                })
            })
        })
        .run()
}
```

在独立工程中执行以下命令：

```sh
cargo run -- list
cargo run -- still intro --time 1.5 --output tutorial-cli.png
cargo run -- --help
```

所有子命令与可选标志参数详见[命令行参考手册](../cli.md)。

`Project::run` 返回 `ExitCode`，通过 CLI 退出状态码直接反馈参数错误或导出失败。调用该方法时，`main` 函数的返回值不要使用 `motion::Result`。

## 导出您的第一部视频

在依赖中开启 `ffmpeg` 特性。它通过原生系统级 FFmpeg C 开发库在进程内执行流式编码，绝非调用外部命令行 `ffmpeg` 进程拼接图片。

macOS 环境准备：

```sh
brew install ffmpeg pkg-config llvm
export LIBCLANG_PATH="$(brew --prefix llvm)/lib"
```

底层库需要支持 libx264 与 AAC，并安装好 pkg-config 和 libclang。推荐使用 FFmpeg 7.x/8.x。
Linux 系统同样需要安装对应的开发头文件（`libavcodec-dev` 等）。单单安装命令行播放器不足以保证 Rust 绑定能够顺利编译。

使用仓库内置示例进行验证：

```sh
cargo run --release --no-default-features --features ffmpeg --example motion -- export motion --output tutorial-motion.mp4
```

对于上述独立的 Project 应用程序，直接运行 `export intro` 即可。视频导出与静态渲染完全复用相同的 Timeline 定义，仅仅输出流程不同。
导出默认具备防文件覆盖保护。请查阅 `--help` 并在需要时显式传入 `--overwrite`。

## 浏览 Gallery 展厅与 Studio 工作台

```sh
cargo run -p motion-gallery --bin motion-gallery -- list
cargo run --release -p motion-gallery --features studio,gpu --bin motion-gallery -- studio --demo type-in-motion
```

Gallery 是一个独立的示例包，具备自身的特性选项。`--features studio` 并不代表每个场景都必须走 GPU 渲染。
Studio 每次启动加载一个场景。视口交互与任务队列详见 [12 Studio 工作台](12-studio.md)。

## 核心 API 快速索引

| API 方法 | 核心功能 |
| --- | --- |
| `Timeline::new(size, rate).build()?` | 创建指定逻辑尺寸与有理数帧率的时间轴 |
| `track.scene(duration, callback)` | 向指定轨道添加带有持续时长的场景闭包 |
| `context.sample(&animation)` | 在当前场景局部时间点对动画进行采样 |
| `render_still` | 请求特定时刻的单张带透明通道的位图 |
| `FrameBuffer::save_png` | 将内存帧保存为 PNG 磁盘文件 |
| `Project::timeline` / `run` | 注册具名短片并生成全功能 CLI 命令行程序 |

## 常见踩坑点

- `Rect::new(left, top, right, bottom)` 接收的是边界坐标，**绝非** `(x, y, width, height)`。
- 默认特性包含原生 C 库依赖。若仅学习基础几何与动画，建议使用 `default-features = false` 以求极速编译。
- Debug 模式下的运行性能不能代表最终导出表现。对于正式短片渲染请始终加上 `--release`。
- 字体、音频与图片路径绝不会隐式从网络下载。教程所提及的文件需显式在本地提供。
- 生成静态图片完全不需要启动 Studio 界面，也不需要手动初始化显卡。无头渲染可完全独立于图形桌面环境运行。

## 后续章节

下一章：[02 核心基础](02-foundations.md)。
可直接运行的仓库示例：[motion 示例](../../../examples/motion.rs)、[音频导出示例](../../../examples/audio_export.rs)、[Gallery 展厅指南](../../../gallery/README.md)。
