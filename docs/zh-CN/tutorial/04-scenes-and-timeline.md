# 04 场景与时间轴

## 本章学习目标

将动态动画与具备排版布局能力的场景相结合，并通过多轨道时间轴组织完整的短片结构。基础示例无需开启任何可选特性；文本排版需要 `text` 特性与显式字体环境。

## 最小可运行示例

```rust,no_run
use motion::prelude::*;

fn main() -> motion::Result {
    let first = scene(|context| -> motion::Result<_> {
        let progress = context.sample(&Tween::new(0.0, 1.0, 2.seconds()));
        Ok(Node::group((
            Rect::new(40.0, 40.0, 440.0, 230.0)
                .fill(Color::rgb(20, 80, 70)),
            Circle::new(Point::new(120.0 + 240.0 * progress, 135.0), 24.0)?
                .fill(Color::WHITE)
                .z_index(1),
        ))
        .opacity(0.9))
    });
    let second = scene(|context| -> motion::Result<_> {
        Ok(Circle::new(context.center(), 40.0)?.fill(Color::rgb(210, 80, 105)))
    });
    let timeline = Timeline::new(Size::new(480.0, 270.0), fps!(30)).track(|track| {
        track
            .push(first.with_duration(2.seconds()))
            .push(second.with_duration(1.seconds()))
    }).build()?;
    render_still(&timeline, 2500.milliseconds().after_origin(), &RenderOptions::default())?
        .save_png("tutorial-timeline.png")
}
```

第二个场景从全局 2 秒时刻开始。当全局时间推进到 2.5 秒时，该场景的局部时间恰好为 0.5 秒。

## scene、SceneContext 与 Node

`scene` 函数为闭包提供上下文类型推断。`Scene` 核心抽象通过 `render(&SceneContext) -> Node` 描述画面；常规闭包支持直接返回基础形状、Node 节点、异构元组或受支持的 `Result` 返回值。

`Node` 是不可变的画面快照。克隆并修改某一个分支绝不会影响其他分支。几何图形、文本、位图图像与自定义 Drawable 均统一挂载在同一棵场景树中。

| API 方法 | 核心功能 |
| --- | --- |
| `Node::group(tuple_or_iterator)` | 组合异构元组或同构迭代器为节点组 |
| `group!` | 简明宏，将多个子元素直接打包为 Node 分组 |
| `Node::push` | 向写时复制的分组中追加子节点 |
| `translate` / `scale` / `rotate` | 局部几何空间变换 |
| `opacity` / `blend_mode` | 图层不透明度与混合模式 |
| `z_index` | 控制同级兄弟节点的绘制覆盖层级 |
| `clip` / `clip_with` | 覆盖率裁剪或几何裁剪 |
| `filter` | 挂载有序的子树滤镜链 |

同级子节点按照稳定的 Z-index 与插入顺序依次绘制。Z-index 不会改变排版布局测量顺序，也不能实现跨祖先层级的全局深度排序。

隐藏在不可见分支下的无效几何依然会被严格校验；降低不透明度绝不是剔除损坏节点的逃生通道。SceneContext 保持局部/全局时钟、资源快照与缓存身份显式清晰。`shifted`、`during` 与 `embed` 建立了明确的时钟边界，而非依赖隐式可变的回放游标。

## 组不透明度与子节点不透明度的显著差异

向整个 Group 节点应用 `.opacity(0.5)` 会先将组内所有子节点完整绘制为一个图层，随后整体应用 50% 不透明度。若将 `.opacity(0.5)` 分别施加给相互重叠的子节点，子节点之间会发生半透明相互透射与颜色混合，产生截然不同的视觉效果。

对于需要整体淡入淡出的视觉组件，应当向外层 Group 统一应用不透明度。仅当元素需要彼此独立淡入时，才分别对各个子节点设置不透明度。

## 布局排版与局部缓存

`Row`（水平行）与 `Column`（垂直列）容器按照插入顺序依次排列测量经过变换的子节点。文本排版使用其真实的排版基准线；几何图形具备自身的基线回退机制。

`context.measure`、`next_to` 与 `align_to` 必须在统一的字体与绘制环境下执行。相对布局计算严格依赖参考节点的测量结果，切勿盲目猜测文本宽度或使用另一套字体环境的数据。

将固定不变的装饰性元素放入 `context.memo(key, builder)` 中，可在每一帧复用静态子树，同时继续对动态参数执行动态求值。缓存键必须覆盖所有决定计算结果的输入变量（包括资产版本、字体、颜色与几何）。缓存被逐出或产生线程竞争时构建闭包可能会被重新执行；切勿依赖仅执行一次的副作用。

## Timeline 时间轴与 Track 轨道

`Timeline::new(size, rate)` 启动 `TimelineBuilder`；调用 `build()` 将其冻结为不可变的、经过验证的 `Timeline`，内部持有设计画布尺寸、有理数帧率、各轨道、`Soundtrack` 原声带与 `diagnostics()` 诊断列表。同一轨道内的片段（Clip）默认按时间先后顺序排列；不同轨道上的片段在同一时刻叠加合成。

| 轨道操作 API | 用途与说明 |
| --- | --- |
| `track.scene(duration, callback)` | 创建带有固定时长的独立场景片段 |
| `track.push(scene.with_duration(duration))` | 引入并复用现有场景 |
| `track.gap(duration)` | 插入一段显式的空白时间间隙 |
| `track.place(start, scene)` | 在轨道的绝对时刻点摆放场景 |
| `track.transition(effect, duration)` | 在相邻片段之间构建转场过渡效果 |
| `TimelineBuilder::audio` | 挂载全局音频播放事件 |
| `TimelineBuilder::duration` | 显式设定整部短片的最终片长；若未指定则自动取所有轨道和音频的结束时间最大值 |

转场（Transition）仅允许两个相邻片段产生时间重叠；任意三层重叠不属于转场的设计范畴。`CrossFade`（淡入淡出）、`Slide`（推移滑动）、`Push`（推挤）、`Wipe`（划像擦除）、`Iris`（光圈缩放）与 `ZoomBlur`（缩放模糊）提供多种合成视觉手法。转场过渡期间两端片段各自保留自身的局部时间轴。

不带 `try_` 前缀的轨道和 Builder 方法会自动收集创作错误，并在 `TimelineBuilder::build` 时统一汇总报错。对于需要遇错立即中止的自动化脚本，请使用 `try_track`、`try_push` 等对应方法。切勿忽视收集到的错误。

片段边界以有理数帧 ticks 独立对齐，若出现未对齐情况会在 `Timeline::diagnostics()` 中记录警告；若显式声明的片长短于内容，会记录 `Truncated` 警告并仅裁切画面。左闭右开的终点时刻不等于最后一帧的整数帧索引。动画位图的时间选择与时间轴摆放解耦；裁剪或倒放视图共享源帧，不重复计算端点时长。

## 嵌套时间轴、摄像机、骨骼与空间坐标

`Timeline` 明确不是 `Scene`。通过 `track.nest(child)`（或 `nest_for(duration, child)` / `place_timeline(start, child)`）将子时间轴作为片段摆放：其画面在居中包含映射前先裁切子视口，其音频在构建期合并入父级原声带。在场景内部，`context.embed(&child.visual(), offset)` 仅嵌入画面，舍弃音频的行为在调用点一目了然。子时间轴的不同帧率不会隐式重置父级时钟，也不会自动重定时所有内容。

`camera(context, content, view)` 向上级子树施加逆视口变换矩阵。保持 HUD 等固定 UI 位于摄像机节点之外，并使用 `ClipMode::Geometric` 约束固定视口。`Camera` 的世界坐标与屏幕坐标互转排除外层祖先变换，且要求矩阵必须为有限且可逆。

`Skeleton`、`Bone`、`BonePose`、`Pose` 与 `BoneTransform` 通过层次化的 f64 空间变换将动画通道连接到常规的 Node 画面。这是 2D 骨骼分层动效，而非 3D 模型渲染。在将动画通道映射到画面前应首先定义局部骨骼坐标；严禁将相同的变换矩阵同时施加给画面与骨骼。

## Project 边界定位

`Project` 负责管理短片 Timeline 注册以及自动生成 CLI 命令行入口；它不是 `Timeline` 的替代品。导出视频、生成静帧与 Studio 视口调试共享完全相同的短片定义，确保预览与导出完全对齐。

## 常见踩坑点

- 误将全局绝对时间当作局部时间传入动画，会导致片段在轨道上移动位置时动画时序错乱。
- 隐藏无效节点无法绕过合法性校验；不可见的分支同样会被严格核验。
- Z-index 仅控制同级兄弟节点的绘制覆盖层级，绝不改变排版布局测量顺序。
- 将固定的 HUD 界面置于 Camera 摄像机内部会导致其跟随镜头一起移动、旋转或缩放。
- 创作期缓存仅用于存储纯函数计算结果，切勿将其当作任意副作用执行器。

## 后续章节

下一章：[05 绘制与图层](05-drawing.md)。
设计契约与源码参考：[场景与快照契约](../../zh-CN/design/core.md#scene-场景与-node-快照)、[时间轴底层实现](../../../crates/motion-core/src/authoring/timeline.rs)、[Project 范例](../../../examples/motion.rs)。
