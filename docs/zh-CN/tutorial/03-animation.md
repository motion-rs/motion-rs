# 03 动画系统

## 本章学习目标

抛弃逐帧手动累加状态的旧思维，学习在显式时间点对动画函数进行无状态采样，并组合位移、静止保持、缓动曲线与连续信号。
基础动画不需要任何可选特性。内容层矢量 Morph 变形需要 `vector` 特性；数学公式 Morph 需要 `formula` 特性。

## 最小可运行示例

```rust,no_run
use motion::prelude::*;

fn main() -> motion::Result {
    let movement = Tween::new(60.0, 420.0, 2.seconds())
        .easing(Easing::QuadraticOut)
        .ping_pong();
    let opacity = Keyframes::new(0.0_f64)
        .to(500.milliseconds(), 1.0, Easing::Linear)
        .hold(3.seconds())
        .to(4.seconds(), 0.0, Easing::Linear);
    let timeline = Timeline::new(Size::new(480.0, 270.0), fps!(30)).track(|track| {
        track.scene(4.seconds(), move |context| -> motion::Result<_> {
            Ok(Circle::new(Point::new(context.sample(&movement), 135.0), 24.0)?
                .fill(Color::rgb(20, 150, 125))
                .opacity(context.sample(&opacity)))
        })
    }).build()?;
    render_still(&timeline, 1.seconds().after_origin(), &RenderOptions::default())?
        .save_png("tutorial-animation.png")
}
```

您可以按任意顺序请求任意时刻的画面：动画始终是对同一时间函数的采样。`ping_pong()` 将一个 2 秒的正向运动扩展为 4 秒的往返运动；最终的关键帧同样精准结束在 4 秒时刻。

## 动画抽象与插值机制

`Signal<Value>` 接收显式的局部 `Time` 并求值返回结果，在时间轴上无始无终。`sample_at(duration)` 是 `sample(duration.after_origin())` 的简写。`FiniteAnimation<Value>` 是具备非负 `duration()` 时长的 `Signal`；`Tween`、`Keyframes`、`FollowPath` 以及后文介绍的时间组合子均属于有限动画。而 `Spring`、`Oscillator`、`Noise`、`Constant` 以及普通的闭包函数仅属于无限 `Signal`。
`Interval { start, duration }` 统一将 `Time` 映射到钳制进度（`progress`）或缓动进度（`ease`）；`Tween`、`SceneContext::progress_in` 与 `ease_in` 均基于此实现。
`Interpolate` 定义两个值如何在 f64 进度下执行插值。它绝不会强行将所有进度粗暴钳制到 `[0, 1]`；弹簧与特定缓动有意保留超出边界的冲冲效果（Overshoot）。

| 动画类型 | 最佳适用场景 |
| --- | --- |
| `Constant` | 随时间保持恒定不变的值 |
| `Tween` | 两端点之间的平滑插值过渡 |
| `Keyframes` | 绝对局部时间点、静止保持、分段缓动 |
| `Spring` | 基于物理参数的弹性阻尼信号；`settle_duration()` 告知其完全静止的时间 |
| `Oscillator` | 连续的正弦波、三角波、方波或锯齿波周期信号 |
| `Noise` | 带随机种子的平滑连续 Perlin 噪声 |
| `FollowPath` | 沿贝塞尔路径按弧长采样的坐标、切线与旋转角 |

`context.sample(&animation)` 默认使用当前场景的局部时钟。若需在其他时钟下求值，请调用 `animation.sample(explicit_time)`，严禁隐式捕获上一帧的旧时间。

## 缓动曲线与关键帧

`Easing::Linear` 产生匀速运动；`QuadraticOut` 实现平滑减速。系统同时提供阶梯步进（Steps）、三次贝塞尔（CubicBezier）与自定义缓动（Custom），但参数与计算结果必须保持为有限浮点数。贝塞尔缓动在读取 Y 轴之前先对 X 轴求解方程，时间绝非直接充当曲线参数。

关键帧的 `.to(time, value, easing)` 接收绝对局部时刻，而 `.hold(duration)` 增加相对保持时长。上述示例在 0.5 秒抵达 1.0，保持至 3.5 秒，并在 4.0 秒抵达 0.0。

目标关键帧决定该时间段所使用的缓动算法。重复或逆序递减的目标时刻属于非法输入，系统绝不会自动对其重新排序。在关键帧精确时刻采样严格返回其保存的设定值。

内置缓动固定锚定在端点。自定义缓动仅钳制其输入进度，但完整保留回调返回的端点数值，中间数值允许外推。非有限输出会显式报错。输入校验无法充当沙箱来拦截用户自定义闭包内部的 panic。

## 时间组合子 (Composing Time)

| 组合子方法 | 含义与使用约束 |
| --- | --- |
| `map` | 将采样出的值映射为另一种类型（适用于任何 `Signal`） |
| `zip` | 将两个信号同步采样并组合成元组；仅当两者均为有限动画时结果才为有限动画 |
| `delay` | 在头部追加一段静止延时 |
| `speed` | 传入有效缩放系数调节时间流速 |
| `then` | 顺序串联两个有限动画 |
| `reverse` | 将有限动画完全倒放 |
| `repeat` | 设定总播放循环次数，而非额外追加的重复次数 |
| `ping_pong` | 先正放后倒放，独立于上一帧状态 |
| `boxed` / `boxed_finite` | 抹除具体静态类型，封装为 `Arc<dyn Signal>` 或 `Arc<dyn FiniteAnimation>` |

`delay`、`speed`、`then`、`reverse`、`repeat` 与 `ping_pong` 仅存在于 `FiniteAnimation` 之上，将其作用于无限信号在编译期直接报错。持续时长为 0 的 Tween 在其边界前后依然具备明确的端点语义，绝非被丢弃的空动画。

`repeat` 统计的是总播放遍数；设置为 0 会永远保持初始值。非法的流速系数与时间溢出会被直接拒绝。对于外部动态输入，请使用带前缀的校验形式（如 `try_delay`、`try_speed`、`Keyframes::try_to` 与 `try_hold`、`Easing::try_apply`、`Duration::try_div`），而非在参数非法时触发 panic 的便利性方法。Duration 类型在设计上强制非负，因此无需进行负时长检查。参数化缓动曲线通过受校验的构造器构建（`Easing::back_in(overshoot)?`、`elastic_out`、`steps`、`cubic_bezier`）；`apply` 调用时不再重复校验参数。

`remap` 与 `piecewise` 支持 Clamp（钳制）、Extend（外推）与 Wrap（回绕）。输入节点必须为有限数值且严格单调递增；格式错误的映射会被 `try_remap` 与 `try_piecewise` 直接报错拦截，绝不静默自动对节点排序。`FollowPath` 基于既有的弧长几何返回坐标、切线向量与旋转角。Noise 噪声与粒子系统的 NoiseDrift 共享确定性底层图元。

## 弹簧与连续周期信号

```rust,no_run
use motion::prelude::*;

fn main() -> Result<(), ValidationError> {
    let spring = Spring::builder(0.0_f64, 100.0)
        .mass(1.0)
        .stiffness(120.0)
        .damping(18.0)
        .settling_threshold(0.0001)
        .build()?;
    let pulse = Oscillator::builder(Waveform::Sine, 1.0)
        .amplitude(0.25)
        .offset(0.75)
        .build()?;
    let drift = Noise::builder(42).frequency(0.5).octaves(3).build()?;
    let time = 500.milliseconds().after_origin();
    let _ = (spring.sample(time), pulse.sample(time), drift.sample(time));
    Ok(())
}
```

`Spring`、`Oscillator` 与 `Noise` 统一通过 Builder 模式进行精细配置；setter 方法从不 panic，`build()` 负责捕获所有非法或溢出参数并返回 `ValidationError`。弹簧的质量（mass）、刚度（stiffness）与阻尼（damping）必须为正数。保守的衰减包络线决定了 `settle_duration()`；从该时刻起，采样值严格恒等于目标值。`settling_threshold` 是相对位移包络阈值，而非绝对像素误差。`initial_velocity`（初速度）以每秒位移占总距离的比例衡量，而非物理像素/秒。

保守的静止时长基于 ticks 取整对齐，绝不是从前一帧的位移中推导出来的。

对于无阻尼的永续振荡，请直接使用 `Oscillator` 或普通数学闭包，而不要试图强求弹簧没有结束时刻。若要为弹簧添加延迟，请在平移后的时间点采样：`|time: Time| spring.sample(time - delay)`。

## Morph 变形的设计定位

纯几何层面的 `MorphPath` 与内容层面的 `Morph` 是两个不同层级的抽象。前者仅负责路径顶点的数值插值；后者负责处理完整的复杂矢量内容。针对数学公式的 `FormulaMorph` 还会额外处理符号选择与抽象语法树的结构对应。

在应用变形前，必须首先规划哪些对象彼此对应、填充规则如何保持以及包围盒如何衡量。两段输入的 SVG 路径数据并不必然拥有兼容的顶点顺序、空洞拓扑结构。

本章重点掌握时间的抽象与组合。[06 内容与资源](06-content.md)将介绍内容级矢量 Morph；[08 Typst 与数学公式](08-formula-typst.md)将介绍公式级变形。

## 常见踩坑点

- 严禁在求值闭包内手动累加坐标状态；这会导致倒退 Seek、掉帧或多线程并发渲染时产生完全错误且不可预测的画面。
- 误将 `.to` 的时间当成相对时长会导致生成重复或逆序递减的关键帧而报错。
- 盲目对所有插值过程进行 `[0, 1]` 范围钳制会抹杀弹簧和部分缓动精心设计的超调回弹弹性感。
- 便利性构造函数并非全部返回 `Result`。遇到不可信外部输入时，请选用带 `try_` 前缀的校验接口以防 panic。
- 无限信号不能自行定义短片的输出总时长。时间轴始终需要显式的 Scene 或总片长设定。

## 后续章节

下一章：[04 场景与时间轴](04-scenes-and-timeline.md)。
设计契约：[无状态动画设计指南](../../zh-CN/design/core.md#无状态动画体系)、[弹簧数学实现](../../../crates/motion-core/src/animation/spring.rs)。
范例短片：[Room to Move](../../../gallery/src/demos/type_in_motion.rs)，封面字素便采用了弹性弹簧动画。
