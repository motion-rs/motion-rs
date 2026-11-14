# motion-rs

[English](README.md) | [简体中文](README.zh-CN.md)

Rust 可编程动态图形与确定性视频渲染引擎。
Cargo package 名为 `motion-rs`；代码中的库导入名为 `motion`。

渲染流水线为：`SceneContext -> Scene -> Node -> DisplayList -> ValidatedDisplayList -> PreparedCpuFrame -> CPU/GPU -> FrameBuffer/Encoder`。无头核心不包含浏览器引擎、UI 控件树、回放时钟，也绝不进行隐式的系统字体扫描。可选的 Motion Studio 提供基于桌面的交互式预览与导出工作台。

## 快速上手

需要支持 Rust 2024 版本的编译工具链。默认特性集包含 GPU 支持、多媒体内容、原生系统级 FFmpeg 以及 Studio 工作台。纯几何无头构建使用 `--no-default-features`，完全不依赖 FFmpeg、字体、GPU 或窗口环境。

```rust,no_run
use motion::prelude::*;

let timeline = Timeline::new(Size::new(640.0, 360.0), fps!(30))
    .track(|track| track.push(scene(|context| {
    let horizontal = context.sample(&Tween::new(32.0, 480.0, 3.seconds()));
    Node::from(Rect::new(horizontal, 64.0, horizontal + 96.0, 88.0)
        .fill(Color::rgb(20, 150, 125)))
    }).with_duration(3.seconds()))).build()?;
let bitmap = render_still(&timeline, 1500.milliseconds().after_origin(), &RenderOptions::default())?;
bitmap.save_png("frame.png")?;
# Ok::<(), motion::Error>(())
```

`Rect::new` 接收对角顶点坐标，**绝非** `(origin_x, origin_y, width, height)`。几何坐标与动画进度统一采用 f64 双精度浮点；时间采用精确整数 ticks（`Time` 为有符号时刻点，`Duration` 为非负时间跨度）以及有理数 `FrameRate`。颜色内部存储为有限的扩展 sRGB 通道；渲染内部使用线性预乘色彩工作空间。直通 RGBA8 格式的 `FrameBuffer` 保留透明通道。`Composition` 及其不透明帧输出属于显式的底层渲染 API，位于日常创作 prelude 之外。

```sh
cargo run --example motion -- list
cargo run --no-default-features --example motion -- list
cargo run --example motion -- still motion --time 1 --output motion.png
cargo run -p motion-cli -- still examples/documents/intro.json --time 1
cargo run --features ffmpeg --example motion -- export motion --output motion.mp4
cargo run -p motion-cli -- still examples/documents/intro.json --time 1
cargo run -p motion-cli -- validate examples/documents/intro.json
cargo run -p motion-gallery --bin motion-gallery -- list
cargo run -p motion-gallery --features studio,gpu --bin motion-gallery -- studio --demo type-in-motion
```

独立的 [Gallery 展厅](gallery/README.md) 包含了综合运用多项特性的完整短片范例。简短的 [motion 示例](examples/motion.rs) 演示了 Project 与 Timeline 的用法；它不是沉重的二次框架抽象。导出命令对已有目标文件提供防覆盖保护。

## Feature 特性矩阵

| 特性 | 功能说明与依赖关系 |
| --- | --- |
| 默认 Default | `gpu,image,svg,text,particles,ffmpeg,studio,ink,diagram,captions,declarative`；场景渲染后端的选择仍保持显式 |
| `vector` | 共享矢量树、Morph 变形与 Ink 墨迹，不开启图片解码或字体排版 |
| `image` | 静态/动态位图图像解码与图像内容呈现 |
| `svg` | 静态 SVG 矢量导入；包含显式的文本与图像准备能力 |
| `text` | 显式字体环境管理、文字塑形与段落排版 |
| `particles` | 确定性物理模拟、运动轨迹与图元实例渲染 |
| `ink` | 艺术化矢量毛笔笔触与宣纸材质浸染；包含图像支持 |
| `diagram` | 统计图表、坐标比例尺、引出标注与刻度尺；包含文本支持 |
| `captions` | 时间轴字幕轨道与时间线索表；包含文本支持 |
| `declarative` | 版本化 JSON 短片文档、JSON Schema 规范及 CLI 读取 `.json` 路径支持 |
| `audio` | PCM 解码、重采样、混音总线与波形分析，无需物理播放硬件 |
| `gpu` | 原生 wgpu 显卡图层合成与外部纹理输出；默认后端，且绝不静默回退至 CPU |
| `typst` | 受信任的单页 Typst 专业数学排版转矢量轮廓/Node |
| `formula` | 直接数学公式创作、子式提取、动态书写与 Morph 变形 |
| `ffmpeg` | 原生 H.264/AAC 视频流式编码与事务性原子落盘 |
| `studio` | 基于 egui 的交互式工作台、视口预览、音频监听与多任务导出队列；包含 FFmpeg |

FFmpeg 导出依赖本地系统安装带 libx264 与 AAC 支持的 C 开发头文件/动态库，以及 pkg-config 与 libclang（用于 [crates/motion-ffmpeg](crates/motion-ffmpeg) 中的第一方 Rust 绑定）。系统绝不启动外部 `ffmpeg`/`ffprobe` 可执行文件进程，也不使用任何第三方 FFmpeg 胶水库。在 macOS 上可通过 `brew install ffmpeg pkg-config llvm` 完成安装；必要时将 `LIBCLANG_PATH` 环境变量指向 LLVM 库目录。经过完整测试的版本为 FFmpeg 8.x。Studio 桌面工作台需要原生图形显示环境。

默认特性使 GPU 和 Studio API 在编译期可用并默认选择 GPU 后端；但它们绝不会自动打开桌面窗口。若需使用参考级软件光栅化器，请显式指定 `BackendPreference::Cpu`。在初始化完毕、遇到不支持的路径或显卡设备丢失后，选定的 GPU 后端绝不会静默回退至 CPU。

## 文档指南

- [教程指南](docs/zh-CN/tutorial/README.md)：第一帧静帧、动画制作、场景时间轴、内容、音频与导出
- [文档导航索引](docs/README.md)：学习、设计与维护指南中英文索引
- [设计契约](docs/zh-CN/design/README.md)：系统架构、渲染、导出、音频、资源、GPU 所有权与可靠性
- [命令行 CLI](docs/zh-CN/cli.md)：`list`、`still`、`export`、`studio` 与 `schema` 子命令与参数手册
- [Studio 用户指南](docs/zh-CN/studio.md)：视口预览、实时回放与导出工作台
- [Gallery 展厅](gallery/README.md)：完整的示范短片作品

精确的公开 API 契约与可运行示例位于 [Rustdoc 源码](crates/motion/src/lib.rs) 中。
可通过 `cargo doc --no-deps --all-features` 本地生成文档网站。

## 开发与维护

引擎实现在各专属子包中完成；`crates/motion` 负责统一组装公开 API。
运行常规测试套件：`cargo test --workspace --all-features --lib --bins --tests --examples`；
运行文档测试套件：`cargo test --workspace --all-features --doc`。
依赖特定字体、物理 GPU 或 FFmpeg 测试固件的用例需单独指定执行；被忽略的测试不能证明在未经测试的物理机上具备运行时支持。

更多细节参见[开发与维护检查](docs/zh-CN/development.md)（了解特性/固件验证流程）、[系统可靠性](docs/zh-CN/design/reliability.md)（了解验收边界）以及[开发规划](docs/zh-CN/plan.md)（了解后续工程规划与待办事项）。
