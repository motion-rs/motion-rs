# 07 粒子系统

## 本章学习目标

创作支持时间轴任意跳转寻址（Seekable）的粒子特效，清晰区分运动轨迹、物理模拟、视觉样式与图元实例渲染层级。本章需要开启 `particles` 特性；显式 GPU 硬件渲染还需要开启 `gpu` 特性。

## 最小可运行示例

```rust,no_run
use motion::prelude::*;
use motion::particles::{Ballistic, ParticleStyle, ParticleTrajectory};

fn main() -> motion::Result {
    let firework = ParticleTrajectory::new(
        Ballistic::burst(Point::new(240.0, 80.0), 200)
            .gravity(Vector2::new(0.0, 180.0))
            .drag(0.8),
    )
    .style(ParticleStyle::GlowDots { radius: 2.0 });
    let timeline = Timeline::new(Size::new(480.0, 270.0), fps!(30))
        .track(|track| track.push(firework.with_duration(3.seconds()))).build()?;
    render_still(&timeline, 1.seconds().after_origin(), &RenderOptions::default())?
        .save_png("tutorial-particles.png")
}
```

建议先从少量粒子开始调试以掌握其生命周期，随后再逐步增加发射密度。短片导出复用完全相同的 Timeline，无需编写手动的逐帧更新循环。

## 粒子系统的四层架构

| 架构层级 | 代表类型 | 核心关注的问题 |
| --- | --- | --- |
| 状态层 (State) | `Particle` | 在特定粒子年龄下，粒子的空间坐标、视觉外观与有限浮点状态是什么？ |
| 解析轨迹 (Analytic Trajectory) | `TrajectoryModel` / `ParticleTrajectory` | 粒子的完整物理状态是否可以在任意时刻无需模拟直接闭式计算？ |
| 定步模拟 (Fixed-step Simulation) | `ParticleSimulation` | 有序排列的发射器与力场算子如何随时间演化粒子的物理状态？ |
| 外观呈现 (Presentation) | `ParticleStyle` / `InstanceBatch` | 求值后的抽象粒子状态如何具体绘制为圆点、光晕、Sprite 精灵图或拖尾线段？ |

轨迹绝不是任意叠加的外力场。`Ballistic`（弹道）同时闭式求解重力与空气阻力；`Converge`（汇聚）靶向目标点集；`Orbit`（轨道）计算圆周相位；`NoiseDrift`（噪声漂移）返回平滑可预测的位移与瞬时速度。

## 随机寻址与物理模拟

解析轨迹在显式的时间、年龄与模型参数下，为每个稳定 ID 的粒子直接计算状态。自定义的 `TrajectoryModel` 应当遵循相同的纯函数契约，严禁在 `state` 回调内部基于上一次的调用结果修改全局可变状态。

物理模拟要求显式提供随机种子、有限时长、发射器列表、有序修饰算子与 `SimulationLimits` 配额。其内部固定采用 `1/240` 秒的时间步长。对任意时刻的查询均精准映射到包含该时刻的固定离散时间步，而非浮动的回放游标。

`ParticleSimulation::new(seed)` 启动 `ParticleSimulationBuilder`；调用 `build()` 将其冻结为延迟准备的不可变模拟（`to_builder()` 可重新打开配置）。应当在场景求值闭包之外构建模拟，或者利用 `SceneContext::try_cache` 将其缓存，缓存键必须完备覆盖所有配置参数与时长。向前或倒退 Seek 会稳定重现完全相同的物理状态；内存配额准入同样会约束缓存快照的存储间距。

## 发射器与修饰算子的顺序敏感性

| 分类 | 常见组件 |
| --- | --- |
| 发射器 (Emitters) | Point（点源）、Rect（矩形）、Disk（圆盘）、Path（路径）、Surface（表面）、Burst（瞬时爆发） |
| 力场与算子 (Modifiers) | Gravity（重力）、Wind（风力）、Drag（阻力）、CurlNoise（旋度噪声）、Attractor（引力源）、Seek（追踪）、Vortex（旋涡） |
| 物理碰撞体 (Boundaries) | BounceBox（弹跳盒）、SdfCollider（有向距离场碰撞体） |
| 生命周期属性曲线 | SizeOverLifetime（尺寸曲线）、ColorOverLifetime（色彩曲线） |

算子的书写顺序直接决定画面的物理行为。在位移之后处理碰撞与先处理碰撞截然不同。粒子寿命与颜色色标必须具备显式的合法范围，系统不会对其进行自动修正或静默排序。

## 图元实例渲染边界 (Instance Rendering Boundary)

运动轨迹、噪声计算与自定义发射器/修饰算子全部在 Rust 侧完成求值。GPU 仅接收计算完毕的坐标、半径、颜色、旋转角度、图元类型与线段端点；显卡绝不解释运行独立的 GPU 轨迹脚本程序。

`Canvas::instances` 记录共享的 `InstanceBatch`，无需将其展开为繁重的独立普通路径。扩展层的 `InstanceUpload::new` 接收强类型的 Disc、Glow、Sprite 与 Segment 记录（采用 SourceOver 混合；`new_blended` 可指定其他 `InstanceBlend`）。上传头部、48 字节图元以及可选的共享精灵图纹素均经过严格校验与预算控制。

纯 CPU 批次默认不向显卡上传数据。显式的 GPU 渲染请求会直接报错拒绝没有 GPU 表示形式的批次，而绝不悄然退化为 CPU。

## 大规模批次仍有物理开销

受支持的 CPU 与 GPU 路径均利用保序的设备空间 Tile 分块来大幅削减每个像素需要遍历的候选粒子集合。它们严格保持粒子的原始发射顺序；禁止使用无序的原子追加操作破坏半透明物体的深度覆盖顺序。

极小规模的批次可能会直接采用全局循环以规避分箱开销。几何边界无法可靠估计的 CPU 批次会自动回退至全量循环路径。GPU 存储、索引容量、像素填充率与 Tile 实例实际工作量在过大时依然可能会触发预算准入失败。

`MAX_INSTANCE_COUNT` 是 `1048576`（100 万）的协议上限，绝非建议的常规粒子数量。模拟步长与发射器配额保持独立限制。Plexus 连线特效使用稳定的空间哈希算法，最多处理 65,536 个粒子状态，并严格限制连线配对数量与计算量。

## 确定性重放与性能准则

固定的随机种子仅解决了随机数输入维度的确定性。时间 ticks、粒子出生时刻、算子顺序、材质资源与后端重放同样必须显式固化。

粒子总数、Tile 索引密度、半透明重叠面积与拖尾长度都会显著影响计算量。强行缩减粒子数量或调大预算会改变画面艺术表现；这不能证明已经达成了 30 fps 的性能指标。

## 常见踩坑点

- 在场景求值闭包内重复构建整套粒子模拟会引发巨大的准备开销，并可能使用错误的缓存键。
- 误用全局秒数替代粒子自身的相对年龄会导致片段在时间轴上移动时粒子生命周期完全错乱。
- 支持 GPU 不代表任何自定义批次都能直接上传；必须具备显式的图元上传格式支持。
- 仅仅限制粒子数量无法彻底消除像素着色与高重叠率开销。少量超大半径的发光点（Glow）依然可能引发显存带宽瓶颈。
- 随意重排半透明粒子的绘制顺序或截断索引数组会破坏画面色彩，绝非无损优化。

## 扩展样式 (Extended Styles)

旧有的 Dots/GlowDots/Sprites/Trails 字面量保持原有的 12 字上传格式不变。可选的 Builder 支持配置 16 字记录格式，同时不改变 `Particle` 状态结构：

```rust
use motion::prelude::*;
use motion::particles::{ParticleCurve, ParticleBlend, ParticleShape, ParticleStyle, TrailOptions};
let trails = ParticleStyle::Trails { length: 120.milliseconds(), width: 2.0 }
    .trails(TrailOptions { segments: 16, taper: Some(ParticleCurve::linear(0.0, 1.0)),
        fade: Some(ParticleCurve::linear(0.0, 1.0)), smooth: true, ribbon: true })
    .blend(ParticleBlend::Plus);
let confetti = ParticleStyle::Shape { radius: 2.0, shape: ParticleShape::Quad { aspect: 1.5 } }.flip();
```

SourceOver、Plus、Screen 与 Multiply 属于批次级混合策略。拖尾的渐细（Taper）与淡出（Fade）严格按照从老到新的拖尾时间线推进；粒子之间共享采样数据。平滑算法在每个区间最多细分 4 次。Polygon（3..=32 边形）、Ring（圆环）、Capsule（胶囊体）、Soft（柔光）与 Quad（四边形）无需加载纹理即可纯几何绘制。`stretch_by_speed(scale, maximum)` 根据运动速度显式拉伸粒子长度。

SpriteSheet（精灵表）支持基于生命周期、固定帧率、带种子随机或 `custom[0]` 槽位选择子图。SpriteAtlas 在内存预算内打包 1..=64 个等宽等高的数据源；双线性采样在选中的晶格单元内部截断防溢色。`SpriteAtlas::new_padded` 支持混合尺寸，将各图片居中放置在透明均匀的晶格中；`.random(seed)` 赋予每个粒子稳定的纹理切片，`.style(size, align)` 返回挂载该精灵表的完整 Sprites 样式（无需多次 Draw Call 即可实现多纹理混合）。`ProceduralSprite` 通过显式随机种子程序化生成 SoftCircle、Smoke、Star、Ring 与 FireCore 纹理。

散景虚化（Bokeh）消费 `custom[1]` 深度数据，按从远到近稳定排序，支持可选视差与弥散圆（CoC）限制。在焦点处粒子半径保持原样；RGB 通道获得高光/CoC 平方能量补偿，而 Alpha 通道保持覆盖率。这是 2.5D 光圈近似，而非全深度缓冲区。`IntensityOverLifetime` 缩放 HDR RGB 亮度而不乘 Alpha，避免逐帧累积虚假能量。`ParticleCurve`/`ColorCurve` 具备显式的时间戳与缓动；非法超调会报错。色温系统将 1000..=40000 K 物理映射为线性 RGB；ColorField 根据粒子位置或速度直接对渐变采样。

## 发射机制与物理碰撞

```rust
use motion::prelude::*;
use motion::particles::{BurstEmitter, CollisionResponse, Inherit, ParticleEventKind,
    ParticleSimulation, SegmentCollider, SubEmitter};
use std::sync::Arc;
let simulation = ParticleSimulation::new(7).duration(1.seconds())
    .emitter(BurstEmitter::new(Time::ZERO, 4, Point::new(20.0, 5.0)))
    .modifier(SegmentCollider { start: Point::new(0.0, 40.0), end: Point::new(80.0, 40.0),
        response: CollisionResponse { restitution: 0.5, friction: 0.2, kill_on_hit: true } })
    .sub_emitter(SubEmitter { on: ParticleEventKind::Collision,
        emitter: Arc::new(BurstEmitter::new(Time::ZERO, 2, Point::ORIGIN).lifetime(100.milliseconds())),
        inherit: Inherit::Position, probability: 1.0 }).build()?;
assert_eq!(simulation.sample(100.milliseconds().after_origin())?, simulation.sample(100.milliseconds().after_origin())?);
# Ok::<(), motion::Error>(())
```

CircleCollider、SegmentCollider 与 PolygonCollider 使用连续扫动碰撞检测（Swept contacts）。SignedDistanceGrid 预先计算带有微小几何精确边界保护带的有向距离场网格；其缓存复用既有的 `SceneCache`。SdfCollider 强制要求 `speed * step` 严格小于最小网格单元。网格分辨率、准备工作量与字节配额均设有限制，绝不隐式降采样。物理碰撞半径为 `particle.size`，独立于视觉呈现半径。

物理事件严格按粒子 ID 排序，随后依次为 Birth（出生）、Collision（碰撞）、AgeReached（达到特定年龄）与 Death（消亡）。子发射器（Sub-emitter）回调接收局部的零步长与零时间点；属性继承将发射原点平移，并可选附加速度或乘以预乘颜色。嵌套深度、总发射量以及单边界事件数均具备独立配额。`accepts_event` 可过滤生效阶段；递归衍生规则必须能够自然终止，否则触发预算超额报错。仅当模拟使用了重放引擎（由预热、循环周期、时间重映射、倒放查询、多工作线程、群体算子、子发射器或重置颜色的算子激活）时，`events` 才会记录。

`SurfaceEmitter.inherit_color(true)` 保持精确的几何点位与面积密度。`VectorSurface.sample_colored_points` 保留有序的纯色/渐变填充；Alpha 不代表密度。图案画刷（Pattern）明确拒绝颜色继承。`ImageEmitter` 计算 Alpha/亮度累积分布函数（CDF）并继承预乘像素色彩；全透明权重为零的图片报错。`SurfaceEmitter.from_text` 捕获已完成塑形的字形矢量轮廓（需要 `text` 特性）；纯位图文字无法充当矢量填充表面。所有发射器均支持 `.paint(Paint, bounds)` 方法，根据发射点处的画刷颜色为初生粒子着色，而不改变空间坐标或随机数流。`PathEmitter.along_path` 沿路径按归一化弧长采样并沿切线方向赋予初速度。

FlowField 流场直接读取带种子的旋度向量或图像 RG 分量。RadialForce 径向力场支持配置作用半径与衰减模型。Boids 鸟群算法、Spring 弹簧链（开链或闭环）以及粒子间相互碰撞作为可选的群体算子，在独立算子执行前运行，具备稳定的遍历顺序与逻辑工作量约束。

## 旋度噪声与演化烟雾

`CurlNoise { frequency, strength, seed, phase_speed }` 计算二维 Perlin 势能场的解析旋度 `[d potential/dy, -d potential/dx]`。`FlowField::curl(seed, frequency, strength)` 对应相同向量场。两者均提供可配置的 `phase_speed`；时间直接推动势能场平流运动，完全不需要维护易变的可变随机数游标。频率必须为有限正数，采样坐标必须适合 32 位有符号整数晶格。

```rust
use motion::prelude::*;
use motion::particles::{BurstEmitter, CurlNoise, FlowField, ParticleSimulation, ProceduralSprite};
use motion::noise::Fractal;
use motion::render::RasterLimits;
let simulation = ParticleSimulation::new(7).duration(200.milliseconds())
    .emitter(BurstEmitter::new(Time::ZERO, 16, Point::new(4.0, 4.0)).speed(2.0))
    .modifier(CurlNoise { frequency: 0.13, strength: 2.0, seed: 42, phase_speed: 1.0 })
    .modifier(FlowField::curl(19, 0.17, 0.5)).build()?;
assert_eq!(simulation.sample(150.milliseconds().after_origin())?, simulation.sample(150.milliseconds().after_origin())?);
let material = ProceduralSprite::Smoke {
    scale: 3.0, evolution: 0.5, fractal: Fractal::default(),
};
let smoke = material.generate_at(32, 42, 250.milliseconds().after_origin(), RasterLimits::sandbox())?;
# let _ = smoke;
# Ok::<(), motion::Error>(())
```

在 `SimulationLimits` 约束下，每个旋度修饰算子在每次物理更新时计费 96 个工作量单位；计算预算不足会直接报错而非跳过采样。工作线程数量与 Seek 定位顺序绝不改变最终计算出的粒子物理状态。

`ProceduralSprite::Smoke` 在 `perlin3([x, y, time * evolution], seed)` 空间上采样三维分形噪声；空间倍频（Octave）不会错误倍乘时间轴坐标。`scale` 范围为 0.25..=64，`Fractal` 最多包含 8 个倍频，调用 `generate_at(size, seed, time, limits)` 在分配内存前会先行对输出像素、浮点临时/Mipmap 存储与噪声工作量执行准入。尺寸为 2..=512 像素。每次调用返回不可变图像：可直接作为 `ParticleStyle::Sprites` 材质使用，或提前烘焙生成显式的 `SpriteAtlas` 用于帧动画回放。单单改变模拟时间不会重新生成精灵图。

Smoke（烟雾）与 Embers（余烬）官方预设均采用了相同的旋度与烟雾材质。粒子物理模拟与精灵图生成全程在 Rust 侧执行；GPU 负责渲染生成的图像与求值后的图元批次，而非在 GPU 上执行粒子物理模拟。

## 时钟控制与多线程并行

`prewarm` 在时间零点之前预先演化一段前置物理步长；在零点爆发的粒子会在预热期间正常产生。`loop_period` 使用半周期偏移相位与余弦交叉淡入淡出实现无缝循环；两个粒子种群共享 `max_particles` 与查询工作量。固定步长状态保持离散量化，因此无缝循环意味着环绕权重的平滑过渡，而非无限连续积分。

`time_remap(scale, offset, allow_reverse)` 在重放前重映射时间查询。负的缩放系数需要显式开启倒放许可；倒放查询能够使消亡的历史粒子重新复活。在时间轴片段范围之外查询返回空集合。`workers(1..=64)` 将独立的粒子物理更新分摊至多个线程并行计算，同时保证物理事件严格保序（粒子数低于 128 时保持串行；在任意工作线程数下计算出的二进制字节完全一致）。邻域搜索（Plexus 连线、Boids 鸟群、物理碰撞）共享有界空间哈希 `NeighborIndex`，将大规模粒子分块派发并在合并时严格按索引保序串联。碰撞器统一视 `Particle::size` 为物理碰撞半径（详见 `collision.rs`）。发射器与算子从计数器派生的 `Random::for_particle` 流中提取随机数，完全独立于遍历顺序与线程数。浮点物理模拟全程在 CPU 上进行。默认最大粒子数保持为 16,384 以确保向后兼容；显式配额最高可调大至 65,536。

## 官方经典预设 (Presets)

```rust
use motion::prelude::*;
use motion::particles::presets::{Preset, PresetOptions};
let mut options = PresetOptions::new(7, 2.seconds(), Rect::new(0.0, 0.0, 320.0, 180.0));
options.density = 0.5;
let sparks = Preset::Sparks.simulation(options)?;
let customized = sparks.modifier(motion::particles::Drag::new(0.3)).build()?;
# Ok::<(), motion::Error>(())
```

`Preset::ALL` 包含：Sparks（火花）、Embers（余烬）、Dust（尘埃）、Motes（微尘）、Snow（雪花）、Rain（雨滴）、Smoke（烟雾）、Fire（火焰）、Confetti（五彩纸屑）、Fireworks（烟花）、Fireflies（萤火虫）、LogoDissolve（Logo 消散）以及 LogoAssemble（Logo 汇聚）。每个预设配方均返回延迟的 `ParticleSimulationBuilder`；Logo 预设需要配置 `options.logo`。Studio 内部同样注册了相同的预设用于交互式预览与导出。Gallery 中的 `particle_comparisons` 工具能够遍历生成所有 Bloom 开关与色调映射组合的对比图：

```sh
cargo run -p motion-gallery --bin particle_comparisons -- /tmp/motion-particle-comparisons
```

## 后续章节

下一章：[08 Typst 与数学公式](08-formula-typst.md)。纯粒子短片也可以直接跳至 [09 渲染与合成](09-rendering.md)。
设计契约与底层实现：[粒子实例协议契约](../../zh-CN/design/particles.md)、[运动轨迹底层实现](../../../crates/motion-content/src/particles/trajectory.rs)。
范例短片：[Signal Garden](../../../gallery/src/demos/signal_garden.rs)。
