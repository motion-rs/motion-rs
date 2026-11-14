# 05 绘制与图层

## 本章学习目标

掌握形状如何转化为指令流，理解渐变色与描边如何被采样，以及裁剪与滤镜如何构建出隔离图层。理解何时该使用底层 Canvas。本章无需开启任何可选特性；第 06 章将讲解加载图片纹理画刷。

## 最小可运行示例

```rust,no_run
use motion::prelude::*;
use motion::extend::{Canvas, Drawable};
use motion::render::{DisplayList, FilterOperation, ValidatedDisplayList};

fn main() -> motion::Result {
    let timeline = Timeline::new(Size::new(480.0, 270.0), fps!(30)).track(|track| {
        track.scene(1.seconds(), |_| -> motion::Result<_> {
            Ok(Node::group((
                Rect::new(60.0, 60.0, 420.0, 210.0)
                    .fill(Color::rgb(20, 150, 125))
                    .filter(Filter::DropShadow {
                        offset: Vector2::new(0.0, 6.0),
                        sigma: 3.0,
                        color: Color::BLACK.with_alpha(0.35),
                    }),
                Circle::new(Point::new(240.0, 135.0), 72.0)?
                    .fill(Color::rgb(220, 90, 115))
                    .clip_with(Rect::new(180.0, 80.0, 300.0, 190.0), ClipMode::Geometric),)
            ))
        })
    }).build()?;
    render_still(&timeline, Time::ZERO, &RenderOptions::default())?
        .save_png("tutorial-drawing.png")
}
```

阴影属于有序的图层滤镜效果；几何裁剪在量化前先行求交。两者绝不是简单地把形状原有的几何包围盒改写为最终像素包围盒。

## Paint 画刷与 Stroke 描边

| API 概念 | 契约与用法 |
| --- | --- |
| `Paint::Solid` / `Color` | 纯色着色绘制 |
| `LinearGradient` | 线性渐变画刷 |
| `RadialGradient` | 径向与焦点（Focal）渐变画刷 |
| `SweepGradient` | 围绕中心点的角向扫描渐变 |
| `ImagePattern` | 不可变图片纹理、坐标映射、平铺扩展、插值采样 |
| `GradientUnits` | 渐变使用的坐标系单位（用户空间 vs 对象空间） |
| `GradientSpace` | 颜色插值所处的色彩空间 |
| `GradientExtend` | 边界外延模式（Pad 截断延伸、Repeat 重复、Reflect 镜像反射） |
| `StrokeStyle` | 描边宽度、端点样式、连接拐角样式、斜接上限（Miter Limit） |

切勿混淆渐变的几何坐标系与色彩插值空间。缩放节点尺寸时，局部绝对渐变与映射到对象包围盒的渐变会产生截然不同的视觉表现。

描边会展开为有界的、受预算限制的闭合轮廓。原始输入路径与生成的描边轮廓均占用预算；线宽为 0 的描边不会跳过参数校验。`LineCap`、`LineJoin` 与斜接上限直接影响几何外观与保守包围盒估算。

## Canvas 指令录制系统

常规应用程序建议优先使用形状的 `.fill` 与 `.stroke` 方法。自定义内容可以通过实现 `Drawable` trait 并在其 `draw` 方法中消费调用方提供的 `Canvas`。严禁在场景预算控制系统之外创建全局静态录制器。

| Canvas 操作方法 | 作用与说明 |
| --- | --- |
| `fill_rect(bounds, paint)` | 保留矩形填充的快速直通路径 |
| `fill(shape, paint, rule)` | 以显式填充规则录制指定形状 |
| `fill_path(arc_path, transform, paint, rule)` | 录制共享的矢量路径与局部变换映射 |
| `stroke(shape, style, paint)` | 计算并录制描边轮廓 |
| `instances(batch, transform)` | 直接录制已求值的图元实例批次，无需展开为繁重路径 |

Canvas 中记录的首个错误会导致本次录制彻底失败。即使用户回调函数吞掉了该错误返回值，后续追加的合法指令也绝不能被当作一张完整的成功画面。

`FillRule::NonZero` 与 `EvenOdd` 对空洞与自相交有截然不同的拓扑判定。路径拓扑与填充规则是场景创作的输入参数，绝非让 CPU 与 GPU 自行猜测的选项。

## 裁剪 (Clips)、遮罩 (Masks) 与图层混合 (Blending)

`clip(shape)` 将独立光栅化得到的覆盖率直接相乘（覆盖率裁剪）。`clip_with(shape, ClipMode::Geometric)` 在量化前执行多边形求交（几何裁剪），针对矩形有专门的快速路径。

在同一个 Node 上附加后续的几何裁剪会直接替换先前的裁剪。若需实现多个裁剪的交集效果，请使用父子嵌套节点。内容遮罩（Content mask）采用 Alpha 通道或线性预乘的 Rec.709 亮度；它与纯矢量路径裁剪是截然不同的概念。

默认采用 `BlendSpace::LinearRgb` 混合空间。显式的 sRGB 空间合成跨越了对应的色彩编码与解码边界。组不透明度作用于整组图层的最终合并结果；对各个子节点分别赋予 Alpha 绝无法替代组不透明度的视觉表现。

## 滤镜执行顺序 (Filter Order)

`Filter` 描述一系列有序的图像处理操作，如模糊（Blur）、投影（DropShadow）、发光（Glow）与色彩微调（ColorAdjust）。镜头色散与暗角通过 `Filter::Optics(Optics::Lens { .. })` 与 `Optics::LensVignette { .. }` 表达；带随机种子的场景线性噪声为 `Filter::grain(seed, amplitude)`（显示空间的胶片颗粒为 `FilmGrain`）。多次调用 `.filter` 严格按声明顺序链式执行。先模糊后调色与先调色后模糊的画面效果截然不同。

子级滤镜效果先于父级滤镜效果完成。滤镜参数使用设计单位；准备阶段据此推导保守的设备捕获区域。布局排版包围盒与受滤镜光晕影响的像素区域是两个完全不同的概念。

绘制完成的源图像经过滤镜处理、遮罩/裁剪后，再以组不透明度与混合模式完成合成。模糊使用归一化的有界盒式近似算法，对极小 sigma 采用小数权重。半透明边界不会引入任何隐式底色。自定义的 `FilterOperation` 显式声明其引用的输入源；滤镜执行顺序绝非根据源出现的距离远近隐式推测。

运动模糊通过对可重现的子帧时刻多次调用场景构建函数，将线性预乘的各次捕获结果求平均；它绝不是简单对前一帧残留画面的平移拖尾。若需要保持字幕或 HUD 元素边缘锐利，请将其置于被运动模糊采样的子树之外。

快门采样中的空白样本依然保留其均摊权重。采样回调必须消费传入的子帧上下文，而非复用先前已渲染的旧帧。GPU 端隔离的内部工作图层一律使用 RGBA32F 浮点纹理（每像素 16 字节）；外部 RGBA16F 仅在显式声明的输出边界处执行量化。内存准入计算的是工作精度内存，而非最终导出精度。

## DisplayList、校验与预算体系

`Node::compile` 录制并校验不可变的 `ValidatedDisplayList`。底层的 `DisplayList` 支持编辑，但修改后必须重新通过校验。使用 `into_editable` 进行显式修改，或使用 `with_background` 仅更新背景色。

结构化校验严格检查节点数、嵌套深度、指令数、路径有效性与参数合法性。光栅化预算严格约束像素总量、逻辑活跃字节与计算工作量。结构校验通过并不保证后续所需的外部资源、GPU 显卡或内存预算一定足够。

`Budget` 预算系统在发生内存扩展前预先扣除配额。限制阈值衡量的是合法支持路径的逻辑开销，并非用于强行限制不可信第三方回调、底层显卡驱动、外部编码器或物理进程 RSS 内存的绝对硬沙箱。自定义的 `Drawable` 属于受信任的代码。

准备完毕的图像画刷直接引用不可变像素数据；系统不存在延迟执行的外部图像生成闭包协议。

## 高级墨迹画笔 (Extended Ink Brushes)

开启 `ink` 特性并引入 `motion::ink` 模块。`BrushStroke::builder(path, width, paint)` 启动针对多笔画几何与完整材质渲染的 `BrushStrokeBuilder`；在任何画质模式下均可通过 `pressure_curve`（压力曲线）、`caps`（笔端形状）、`bristles`（笔鬃细节）与 `tiled_material`（分块材质）等链式方法进行精细微调，最后调用 `build()` 执行校验并返回不可变的 `BrushStroke`。`BrushStroke::new(path, width, paint)` 直接构建默认笔触。颜色参数支持任何 `Into<Paint>` 类型：普通的 `Color`、渐变色以及图片纹理画刷均可直接传入。烘焙后的材质缓存在已构建的笔触对象内部，因此建议一次性构建并保留笔触引用以便在每次动画显现时直接克隆，切勿在每一帧回调内部重复执行耗时的笔触重建。

```rust,no_run
use motion::prelude::*;
use motion::ink::{BrushStroke, NibMode, StrokeFont};

fn main() -> motion::Result {
    let path = Path2D::builder()
        .move_to(Point::new(20.0, 30.0))
        .line_to(Point::new(120.0, 70.0))
        .build()?;
    let stroke = BrushStroke::builder(path, 8.0, Color::rgb(20, 150, 125))
        .pressure_curve(vec![(0.0, 0.2), (0.5, 1.5), (1.0, 0.1)])
        .nib_mode(NibMode::FollowPath(0.0))
        .bristles(5, 0.4, 0.1, 17)
        .parallel_material(true)
        .build()?;
    let _revealed = Node::from(stroke.clone().write(0.5));
    let _cursor = stroke.write(0.5).tip_node(Circle::new(Point::new(0.0, 0.0), 2.0)?.fill(Color::WHITE))?;
    let _letters = StrokeFont::new(20.0)?.strokes("MOTION", 1.0, Color::WHITE)?;
    Ok(())
}
```

`pressure`、`pressure_curve` 与 `pressure_from` 采用“最后声明者生效”语义。`write(progress)` 根据原始弧长从起点展开显现，并复用已缓存的材质；这与 `Styled::trim` 截取普通描边轮廓有本质不同。`BrushCap`（`Round` 圆头、`Flat` 平头、`Tapered` 锥形聚拢）控制变宽笔刷两端的压力截面包络，而 `LineCap`（`Butt`、`Round`、`Square`）决定均匀线宽描边的轮廓端点拓扑，二者彼此独立。`InputSample` 记录有限局部坐标、0..=4 范围内的压力、弧度制倾斜方位角以及从 0 起算的严格单调递增 `Time`（支持 2..=512 个采样点）。`BrushStrokeBuilder::from_samples` 拟合 Catmull-Rom 样条曲线；`write_with(time)` 采用拟合的弧长/时间映射，完美重现毛笔停顿的飞白驻留。`BrushTiming`/`write_timed` 为独立的子路径提供单调速度曲线与提笔间歇时间线。

`BrushPreset` 预设提供 10 种经典笔触风格。`.stroke(path, width, paint)` 返回可修改的 `BrushStrokeBuilder`；`.node` 直接构建笔触并自动配置推荐的正片叠底（Multiply）、不透明度与 Bloom 泛光合成。`paint` 可替换为纯色、渐变或图案画刷，`bristles` 模拟带随机种子的细腻笔丝，而 `wobble`、带帧种子的 `boil` 以及解析式的 `try_wetness` 均保持完全可重放。含墨量轮廓配置保留在 Builder 上，而材质效果如 `build_up`（墨层堆积）、`edge_anisotropy`（边缘各向异性）、`edge_darkening`（水渍边缘暗沉）与 `height_map`（高度图）属于 `Ink` 全局设置。`tip`/`tip_node` 导出最终子路径的端点坐标与切线方向，用于精准锚定光标或粒子发射器；`erase(content)` 基于反转 Alpha 遮罩擦除笔迹，无需添加额外的底层着色器混合模式。

统一的 `Paper`（宣纸/画纸）枚举通过 `Ink::paper` 同时服务于 `BrushStroke` 笔触与 `InkFill` 水墨浸染：`Paper::Procedural { seed, strength, scale }` 生成带随机种子的程序化纸张肌理，而 `Paper::Texture { texture, grain_scale, absorbency }` 接收调用方持有的解析图像，利用其重复高度采样衰减墨水覆盖率。纸张默认采用未变换的设计坐标；若要实现画布固定的纸张纹理，必须通过 `Ink::paper_transform` 显式抵消外层节点的空间变换。`InkFill` 支持控制干燥度、水晕扩散、粗糙度、颗粒沉降（Granulation）、纸张、水渍沉积与边缘暗沉；而微观纤维、墨层堆积、边缘各向异性与高度图需要矢量描边几何支撑，仅对画笔笔触生效。`Ink::height_map` 使笔触生成一张额外的灰度凸起/墨层高度 Drawable，供 `Optics::Relight` 搭配 `HeightFrom::Luma` 进行立体表面重新打光。

`PigmentMix::KubelkaMunk.painting(strokes)` 采用 6 频段手写反射率近似公式模拟颜料在覆盖交叠处的减色法真实混色；它绝非物理流体模拟。`StrokeFont` 采用专有的 14 段大写字母/数字/正负号单线中心骨骼，自动将小写字母转换为大写，支持空格与换行符并报错拒绝未知字符。它完全不依赖复杂的字体矢量轮廓骨骼化算法，也无需加载外部 SVG 或 TTF 字体文件。

扩展几何支持最多 64 条子路径、每路径 512 个采样区间以及 64 根笔丝，同时受限于全局描边动词预算。材质分块准入 16..=2048 条边（默认 2048 条）、最多 256 个分块、全局 16384 条边/1600 万纹素总量以及有界距离场计算。颜料合成单独准入 32 笔笔触、2048 边/262144 纹素输出与 3200 万次覆盖率求值。串行与并行分块计算位完全一致；CPU 与 GPU 消费完全相同的已烘焙材质数据。Draft 草稿模式与 Final 终版模式下的扩展笔刷完全共享该流水线。

## 常见踩坑点

- 覆盖率裁剪与几何裁剪在像素比特上并不等价；两者在抗锯齿量化顺序上存在细微差别。
- 切勿根据滤镜光晕扩展后的像素包围盒去推导布局对齐。布局与捕获区域服务于截然不同的目的。
- 忽略 Canvas 录制错误并强行返回 Ok 无法清除内部已经产生的错误状态。
- 切勿随意重排滤镜顺序。子树隔离状态与滤镜顺序会直接影响最终画面的色彩与模糊半径。
- 预算数值并不等同于系统物理显存或进程 RSS 硬上限；共享资产与外部强引用不计入其中。

## 后续章节

下一章：[06 内容与资源](06-content.md)。
底层契约参考：[图层与采样契约](../../zh-CN/design/core.md#图层合成与采样)、[系统可靠性](../../zh-CN/design/reliability.md)、[Canvas 源码实现](../../../crates/motion-core/src/drawing/canvas.rs)。
完整自定义 Drawable 示例请参阅 [14 架构与扩展](14-architecture-and-extending.md)。
