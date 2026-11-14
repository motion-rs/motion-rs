# 00 引擎概览与流水线

## 本章学习目标

评估 motion-rs 是否契合您的项目需求，并厘清创作、渲染与输出的分层边界。本章示例无需开启任何可选特性。

motion-rs 是一个面向 Rust 的可编程动效视频图形引擎。场景函数（Scene function）描述特定时刻的画面内容，时间轴（Timeline）组合多轨道内容，输出链路负责生成静态图片或编码视频文件。它不是基于浏览器的 DOM 动画框架，运行时既不需要物理显示窗口，也不需要可变的回放时钟。

## 最小可运行示例

```rust,no_run
use motion::prelude::*;
use motion::render::{DisplayList, ValidatedDisplayList, FrameBuffer, PreparedCpuFrame};
use motion::extend::{Encoder};
use motion::vector::{Morph};
use motion::particles::{Particle};

fn main() -> motion::Result {
    let timeline = Timeline::new(Size::new(320.0, 180.0), fps!(30)).track(|track| {
        track.scene(1.seconds(), |_| -> motion::Result<_> {
            Ok(Circle::new(Point::new(160.0, 90.0), 24.0)?
                .fill(Color::rgb(20, 150, 125)))
        })
    }).build()?;
    let bitmap = render_still(&timeline, Time::ZERO, &RenderOptions::default())?;
    bitmap.save_png("tutorial-overview.png")
}
```

该程序没有维护任何动态回放时钟。`render_still` 直接请求时间零点，场景函数仅按需计算该时刻所需的单一画面。

## 处理流水线模型

从应用程序创作者的视角来看：

```text
SceneContext -> 场景闭包函数 -> Node 节点树
    -> 录制并校验绘制指令 -> ValidatedDisplayList 已校验指令列表
    -> PreparedCpuFrame 已准备帧 -> CPU/GPU 渲染后端 -> FrameBuffer/纹理/Encoder 编码器
```

`DisplayList` 是一个可直接修改的底层指令容器。直接编译普通的 `Node` 会产出 `ValidatedDisplayList`；应用程序开发者无需手动构建每一个中间结构。

| 对象类型 | 核心职责 | 绝不代表 |
| --- | --- | --- |
| `SceneContext` | 局部/全局时间、设计尺寸、外部资源、随机数流、创作期缓存 | 拥有回放时钟或 GPU 命令队列 |
| `Node` | 场景结构、空间变换、图层、视觉内容 | 已经光栅化渲染出的像素数据 |
| `ValidatedDisplayList` | 不可变、经结构校验的扁平绘制指令序列 | GPU 硬件可用或光栅化预算充足 |
| `PreparedCpuFrame` | 设备空间覆盖率、画刷数据源、裁剪与滤镜数据 | 任务已经提交至 GPU 执行 |
| `FrameBuffer` | 显式像素格式的帧数据（包含直通 RGBA8 静帧与 PNG 输出） | 所有格式在内存布局上完全相同 |
| `Encoder` | 顺序消费各帧并执行最终的输出事务提交 | 包含场景构建逻辑或图形用户界面 |

## 核心设计哲学

### 按时间显式求值 (Evaluate by Time)

将画面视为时刻、参数、外部资源与随机数种子的纯函数。跳转至第 100 帧绝不应需要先回放前面的 99 帧。粒子模拟内部可有状态，但其显式的模拟与采样契约必须保证完全可重现。

### 显式配置的运行环境 (Explicit Environments)

纯几何图形渲染无需字体、GPU 或窗口。文本与 SVG 需要明确的字体环境；追求确定性的项目应当注册固定的字体文件或字节。外部资源加载必须提供显式根目录，严禁将相对路径任意当作全局默认路径。

### 确定性是有作用域的 (Determinism Has a Scope)

相同输入下的 CPU 纯算术求值与相同显卡下的 GPU 重放各自拥有专属保证。CPU/GPU 像素容差以及跨设备、字体或编译器工具链的微小浮点差异属于不同维度的考量。相同的随机数种子并不代表在所有操作系统平台上均输出逐比特完全相同的像素。

### 划分清晰的输出边界 (Keep Output Boundaries Distinct)

GPU 内部工作图像与 Studio 视口预览可全程停留在显存中。若需返回 `FrameBuffer` 或 CPU 可读帧，会触发显式的内存回读边界；视频编码过程同样具备格式转换、线程同步与对象生命周期约束。引入 GPU 绝不能消除应用程序侧的所有 CPU 准备工作。

## Feature 特性选择指南

| Cargo 特性 | 所获能力与依赖链 |
| --- | --- |
| 无额外特性 | 基础几何、时间轴、CPU 绘制、基本图片导出 |
| `vector` | 共享矢量内容与 Morph 变形，不开启图像解码或外部字体 |
| `image` | 静态/动态位图图像；隐式包含 `vector` |
| `text` | 文本排版、字体管理、段落折行；隐式包含 `image` |
| `svg` | SVG 导入；隐式包含 `text` 与 `image` |
| `particles` | 粒子轨迹、物理模拟、实例协议；隐式包含 `image` |
| `ink` | 笔刷笔触与纹理材质填充；隐式包含 `image` |
| `diagram` | 统计图表、刻度标尺、引出标注；隐式包含 `text` |
| `captions` | 字幕与时间戳标记；隐式包含 `text` |
| `audio` | 音频解码、混音总线、流式分析；无需物理播放设备 |
| `gpu` | 基于 wgpu 的显卡合成与外部纹理输出；不自动强选后端 |
| `typst` | 受信任的单页 Typst 专业数学排版 |
| `formula` | 直接数学公式创作、子式提取、书写、Morph；隐式包含 `typst` |
| `ffmpeg` | 原生视频与音频编码封装；隐式包含 `audio` |
| `studio` | 交互式桌面工作台；隐式包含 `ffmpeg` |

默认特性集启用了 `gpu,image,svg,text,particles,ffmpeg,studio,ink,diagram,captions`。这是为全功能动效创作提供的开箱即用预设，而非极简依赖集。`formula` 与 `typst` 需要显式按需开启。

## 常见踩坑点

- 开启 `gpu` 特性并不代表自动采用 GPU 渲染。必须通过渲染选项显式指定后端。
- 结构性校验通过并不代表运行时所有条件必然满足。外部资产、预算超额、显卡故障或编码器依然可能在后续阶段报错。
- 严禁在帧求值回调内部读取系统墙钟时间或使用未指定种子的随机数，这会彻底破坏时间轴 Seek 与重放一致性。
- 2D 动效场景明确不是 3D 游戏引擎。当前不提供 3D 模型导入或立体光栅化管线。
- 严禁在每一帧中重复创建整套运行环境。应当在会话间共享固定的资产与字体，并通过 RenderSession 复用渲染状态。

## 后续章节

下一章：[01 快速上手](01-quick-start.md)。
设计契约与源码入口参考：[系统架构](../../zh-CN/design/architecture.md)、[包与特性划分](../../zh-CN/design/architecture.md#包与特性管理)、[最小 Project 示例](../../../examples/motion.rs)。
