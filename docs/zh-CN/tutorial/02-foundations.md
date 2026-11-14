# 02 核心基础

## 本章学习目标

清晰区分时刻、逻辑坐标、色彩空间与物理像素，确保场景设计不被硬编码绑定到特定的帧率或分辨率上。本章无需开启任何可选特性。

## 最小可运行示例

```rust,no_run
use motion::prelude::*;

fn main() -> motion::Result {
    let timeline = Timeline::new(Size::new(480.0, 270.0), FrameRate::NTSC)
        .track(|track| {
            track.scene(2.seconds(), |context| -> motion::Result<_> {
                let horizontal = 48.0 + context.random("marker-position") * 300.0;
                let path = Path2D::builder()
                    .move_to(Point::new(48.0, 200.0))
                    .line_to(Point::new(240.0, 48.0))
                    .line_to(Point::new(432.0, 200.0))
                    .close()
                    .build()?;
                Ok((
                    path.fill(Color::from_hex(0x168C78)),
                    Circle::new(Point::new(horizontal, 210.0), 8.0)?
                        .fill(Color::WHITE),
                ))
            })
        }).build()?;
    let time = Time::from_frame(30, FrameRate::NTSC);
    render_still(&timeline, time, &RenderOptions::default())?
        .save_png("tutorial-foundations.png")
}
```

带种子的随机标记位置完全由其键（`marker-position`）决定。重复求值或向前倒退 Seek 绝不会使其发生跳跃偏移。时间严格从有理数帧率精准计算，而非先粗暴地四舍五入为 30 fps。

## 时间不是浮点秒数的粗暴累加

`Time`（时间轴上的有符号时刻点）、`Duration`（非负时间跨度）与 `TimeOffset`（有符号时间位移）是基于底层整数 ticks 的三个独立类型，每秒固定划分为 `705,600,000` 个 ticks。`Time - Time` 产生 `TimeOffset`，`Time ± Duration` 产生 `Time`，而直接相加两个 `Time` 在编译期直接拒绝。Duration 绝不能构造负值：`1.seconds()` 字面量作为源码常量，负值会触发 panic，而针对外部运行时的输入应当使用 `Duration::try_from_seconds`。秒数仅仅是一个面向物理/几何计算的便捷视图，绝非引擎内部可变的回放状态。帧率支持以宏 `fps!(30)` 书写（传入 0 编译报错），或通过 `FrameRate::try_new(30_000, 1001)?` 构造。

| API 方法 | 核心功能与含义 |
| --- | --- |
| `1.seconds()` / `250.milliseconds()` | 从时间字面量构造 `Duration` 时长 |
| `Time::ZERO` | 时间轴的绝对零点 |
| `duration.after_origin()` / `time.since_origin()` | 在相对时间跨度与从零点起算的绝对时刻点之间显式转换 |
| `Time::from_frame(index, rate)` | 独立精准计算单帧的时间边界 |
| `FrameRate::NTSC` | 有理数帧率 `30000/1001` (约 29.97 fps) |
| `context.time()` | 场景局部相对时间 |
| `context.seconds()` | 场景局部时间的秒数浮点视图，方便带入物理公式 |

始终独立计算每个帧边界的时间，绝不要通过反复累加 `1.0 / fps` 来推进时间。时间轴与字幕区间均采用左闭右开区间：包含起始时刻，排除结束时刻。负时间在时间轴上具备明确的物理意义；不要在每个输入层轻易将其截断为零。

## 逻辑设计坐标与设备物理像素

`Point`、`Vector2`、`Size`、`Rect` 以及 `Transform` 统一采用 f64 双精度浮点。场景尺寸描述的是纯逻辑设计坐标；最终输出的物理像素尺寸使用 `PixelSize`。在 Studio 中缩放视口预览尺寸绝不需要创作者重写任何设计坐标。

```rust,no_run
use motion::prelude::*;

fn main() {
    let bounds = Rect::new(20.0, 30.0, 120.0, 90.0);
    let center = Point::new(70.0, 60.0);
    let displacement = Vector2::new(12.5, -4.0);
    let _ = (bounds, center, displacement, Transform::IDENTITY);
}
```

上述矩形宽度为 100，高度为 60。`Rect::new` 接收对角顶点 `(left, top, right, bottom)`，而非 `(x, y, width, height)`。负尺寸或非法的几何数据绝不会因为长得像矩形就被隐式修复。

`Shape` 抽象统一提供包围盒与矢量路径。如 `Circle`、`Rect`、`Polygon` 与 `Path2D` 等形状通过 `.fill` 填充或 `.stroke` 描边转化为可绘制的图形，随后可直接挂载到 `Node` 树中。

## 色彩空间与 Alpha 通道

| API 方法 | 含义与用法 |
| --- | --- |
| `Color::rgb(20, 150, 125)` | 8 位整数 RGB 输入 |
| `Color::from_hex(0x168C78)` | 十六进制十六位整型 RGB |
| `Color::from_rgb_float(1.5, 0.4, 0.1)` | 有限的扩展 sRGB 输入，不受限于标准 SDR 显示范围 |
| `color.with_alpha(0.5)` | 设定透明度 Alpha |
| `to_rgba8` | 钳制并量化为输出显示的 8 位字节；非 HDR 色调映射 |
| `to_linear_premultiplied` | 转换为渲染器工作时使用的线性预乘颜色表示 |

`Color` 接收 f64 输入并在内部存储为有限的 f32 单精度浮点通道。内部渲染工作图像一律采用线性预乘颜色。在普通绘制中，创作者**严禁**手动将 RGB 乘以 Alpha，否则会导致引擎内部二次预乘变暗。

扩展 sRGB 能够表达比常规纯白更亮的 HDR 有限数值。但这并不代表导出的常规 PNG 或低位深视频能够直接保留高动态范围，也不意味着系统会自动执行 HDR 屏幕色调映射。

Oklab 色彩插值可在限制 Alpha 通道的同时外推 RGB。几何计算完备保留带符号的超调（Overshoot），而非在创作阶段过早转换为整数像素。Rect 矩形包含测试为左闭右开。

公开辅助函数 `srgb_to_linear`、`linear_to_srgb`、`rec709_luminance`、`premultiply` 与 `unpremultiply` 作用于 f32 数值。相对亮度计算要求输入为线性 RGB；Alpha 为零时的反预乘会自动清除残留的未定义 RGB。这些辅助函数不持有画面内存，也不自动执行色调映射。

## 空间变换与矢量路径

`NodeModifiers` 为视觉内容附加位移（Translation）、缩放（Scale）、旋转（Rotation）、轴心（Pivot）与不透明度（Opacity）。默认轴心位于测量得到的几何中心；若要围绕特定点旋转，需显式声明轴心。变换是有顺序的：先缩放后平移与先平移后缩放产生的矩阵截然不同。

`Path2D` 在内存中完备保留线段与高阶贝塞尔曲线，而非过早将其光栅化为像素点。其链式构造方法接收 `Point`（例如 `move_to(Point::new(...))`），而非两个分散的数值参数。

路径同时支持导入 SVG path data 字符串、提取描边外轮廓、路径修建（Trim）以及虚线样式。遇到自相交或空洞轮廓时，请根据需求显式指定 `FillRule::NonZero` 或 `EvenOdd` 填充规则。

对于来自网络或外部编辑器的不可信路径数据，建议使用带前缀的 `try_` 方法以便就地捕获解析异常。链式创作方法内部会保留首个错误状态；后续追加合法的路径线段无法消除先前已经发生的校验失败。

## 确定性随机数系统

`context.random(key)` 返回 `[0, 1)` 范围内的稳定浮点随机数。`context.random_generator(key)` 返回基于该键启动的独立随机数流。应当为每个对象指定业务相关的稳定唯一字符串标识，而非使用在遍历过程中随时可能变动的循环临时索引。

从同一个随机数流中连续提取数值取决于采样先后顺序。对于彼此无关的独立图元，应使用不同的键。稳定的随机数算法无法弥补外部数据文件变动、读取系统时间或修改全局可变状态带来的不确定性。

## 常见踩坑点

- 严禁混淆 `Duration`、`Time` 与整数帧索引；编译器会直接拒绝非法类型混用，请进行显式转换。
- 将 NTSC 帧率粗暴视为 30.0 并在循环中累加浮点秒数，会导致长视频在音频/画面对齐边界产生严重的时间漂移。
- 严禁在场景内部硬编码写死特定的预览窗口物理像素宽度。请始终从 `context` 读取逻辑尺寸或几何信息。
- Infinity、NaN 与不可逆的奇异矩阵属于必须在输入源头拦截的非法数据，系统绝不会对其进行静默纠错。
- 相同的随机种子无法保证跨操作系统、跨显卡硬件产生逐比特绝对一致的画面。字体库微小版本差异与显卡光栅化硬件属于环境输入变量。

## 后续章节

下一章：[03 动画系统](03-animation.md)。
进阶参考：[时间与无状态动画设计](../../zh-CN/design/core.md#无状态动画体系)、[色彩数学实现](../../../crates/motion-core/src/color/math.rs)、[路径实现](../../../crates/motion-core/src/path2d.rs)。
