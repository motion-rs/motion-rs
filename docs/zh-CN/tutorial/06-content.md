# 06 内容与资源

## 本章学习目标

在统一的资源、字体与缓存环境下，向场景中引入位图图像、SVG 矢量与富文本排版。根据项目实际需求开启对应特性；您无需一次性开启所有内容模块。

| 内容模块 | 所需 Feature |
| --- | --- |
| 静态图像与序列动图 | `image` |
| SVG 矢量图形 | `svg` |
| 文本排版与字体管理 | `text` |
| VectorTree 矢量树与 Morph 变形 | `vector` |
| 墨迹笔刷与纸张材质填充 | `ink` |
| 统计图表、刻度尺与标注 | `diagram` |
| 动态字幕与时间轴线索 | `captions` |

## 最小可运行示例：显式字体环境

在依赖中开启 `text` 特性。在仓库根目录下运行该示例将直接使用内嵌的 Noto 字体。在独立工程中，请将路径替换为显式提供的字体文件。

```rust,no_run
use motion::prelude::*;

fn main() -> motion::Result {
    let mut fonts = FontLibrary::new();
    fonts.load_file("tests/fixtures/fonts/NotoSans-Regular.ttf")?;
    let assets = Assets::new(".")?.with_fonts(fonts);
    let options = RenderOptions::default().resources(assets);
    let timeline = Timeline::new(Size::new(480.0, 270.0), fps!(30)).track(|track| {
        track.scene(2.seconds(), |context| {
            Text::new("Motion, one frame at a time.")
                .family("Noto Sans")
                .size(26.0)
                .fill(Color::rgb(20, 150, 125))
                .write(context.progress())
                .translate(32.0, 100.0)
        })
    }).build()?;
    render_still(&timeline, 1.seconds().after_origin(), &options)?
        .save_png("tutorial-content.png")
}
```

`FontLibrary::new` 创建一个绝对纯净的空环境，绝不主动扫描操作系统字体。`load_file` 同步读取并注册显式的字体字节；`Assets::new` 会进行合法性校验，而 `with_fonts` 派生出绑定该字体的快照视图。缓存的唯一性包含字体环境；现存的快照和句柄严格保持各自独立的生命周期。

## 图像与资源所有者机制

`Image::open("photo.png")` 仅保存数据源引用，在编译阶段通过调用方的 Assets 进行实际解析。`ImageAsset` 是固定内容的即时表示；`Image` 则在场景层控制画面呈现、填充方式（Fit）、显示尺寸、圆角与插值采样。

为本示例开启 `image` 特性；无需准备外部图片文件：

```rust,no_run
use motion::prelude::*;
use std::sync::Arc;

fn main() -> motion::Result {
    let assets = Assets::new(".")?;
    let pixels = ImageAsset::from_rgba8(32, 32, vec![255; 32 * 32 * 4])?;
    assets.preload("tile.png", Arc::new(pixels))?;
    let options = RenderOptions::default().resources(assets);
    let timeline = Timeline::new(Size::new(160.0, 90.0), fps!(30)).track(|track| {
        track.scene(1.seconds(), |_| {
            Image::open("tile.png").translate(64.0, 29.0)
        })
    }).build()?;
    render_still(&timeline, Time::ZERO, &options)?
        .save_png("tutorial-preloaded-image.png")
}
```

预加载（Preload）与延迟加载（Lazy load）共享同一个所有者。扩展接口（如 `AssetsExt::image`）返回当前的 `Handle<A>`。创建句柄并不代表已经完成解码；调用 `get()` 才会请求实际的解码内容。

PNG/JPEG/WebP 解码为共享的线性预乘 Mipmap。采样插值、纹理贴图映射与边缘平铺无需在绘制前拷贝像素。`AnimatedImageAsset` 存储合成的 GIF/APNG 或显式的序列帧；`AnimatedImage` 通过 ticks 寻址指定帧。循环与往返播放绝不重复计算端点时长。

## SVG 矢量图形与 VectorTree

SVG 源码直接解析为引擎共享的 `VectorTree`，而非通过另一套隐藏的位图光栅化库间接渲染。`Svg::open` 复用与图像相同的资源所有者。SVG 内部包含的文本与位图同样需要对应的字体与图像资产支持。

| API 类型 | 作用与说明 |
| --- | --- |
| `SvgAsset` / `Svg` | 解析后的 SVG 资产及其在场景中的呈现 Node 节点 |
| `VectorTree` | 共享路径、分组、图像与遮罩的纯矢量中间表示树 |
| `Morph` | 内容级矢量插值变形引擎 |
| `WriteOptions` | 显式准备的矢量手写描边动效配置 |
| `ImagePattern` / `Sampling` | 图像纹理画刷与双线性/临近采样重建 |

目前支持静态 SVG 特性。遇到不受支持的语法请查阅诊断日志或错误提示；切勿假定浏览器环境下的 SVG 动画、DOM 脚本或动态 CSS 同样可用。

## 文本是段落排版，绝非单个字符纹理拼贴

`Text` 与 `TextSpan` 支持字体家族、字重、字号、行高、字间距、排版宽度、文本对齐、富文本与描边轮廓。排版流程包含双向文本（Bidi）、字符回退与字素簇分析；单凭 UTF-8 字节数或字符编码绝对无法推断文字的真实物理渲染宽度。

`ParagraphStyle` 保存完整的段落描述（文本、样式、宽度、对齐、字体）；`Text::paragraph` 暴露出该描述，`Text::from_paragraph` 将其打包为节点。其 `layout` 方法生成 `ParagraphLayout`，内含实际的字形轮廓、基线、断行信息与源码映射，该排版产物被所有输出路径（静态绘制、`write`、`animate_parts`、字幕系统）无缝复用。固定宽度或多行文本应使用该测量结果，切勿使用 `字符数 * 字号` 粗暴计算。

`Text::write` 与 `animate_parts` 可作为延迟效果使用，继承场景局部时间。如需预先自检排版或截断文本，请使用显式的 Eager 或上下文接口。`fit_lines` 属于预计算，切勿在每一帧内部重复执行。

当字体、文本与排版参数未发生变化时，排版布局会被自动复用。仅仅修改文字颜色或位移坐标无需重新触发耗时的塑形排版。不同的字体环境之间绝不混用缓存。

文本分片完备保留字素簇与源码边界、可见性以及基线布局。文本下划线等修饰线应由测量后的文本尺寸结合常规几何图形组合而成，而非另立一套文本渲染器。单个 Text 节点仅挂载一个显现动效：在调用 `animate_parts` 之后调用 `write`（反之亦然）会直接替换先前的动效；若需同时使用手写与逐字空间变换，请将其拆分为独立的 Text 节点。

## 墨迹、图表与字幕工具

`BrushStroke` 融合矢量路径、独立笔尖尺寸、压力曲线、含墨量与弧长展开；通过 `BrushStroke::builder(...).build()?` 进行构建。`BrushStroke` 与 `InkFill` 均接收纯色、渐变或图案画刷，且 `InkFill` 复用相同的确定性 `Ink` 材质与 `Paper`（填充会自动忽略仅对描边生效的纤维与墨层堆叠）。枯笔飞白、宣纸肌理、渗水晕染、粗糙度与水渍积聚属于程序化覆盖率计算，并非物理流体模拟。

应当保留配置好的完整笔触对象，仅通过动画驱动其显现进度。切勿为了规避单次开销而将一条连续笔画拆碎为大量微小片段，这会破坏压力连续性与材质真实感。

`motion::diagram` 提供常用的图表与标注工具，如 Scale（坐标轴）、LineChart（折线图）、Ruler（标尺）与 Callout（标注框）。物理单位采用 f64，图例与标签需要显式绑定字体环境。

`motion::captions` 提供 CaptionTrack 与 CueSheet。时间线索（Cue）采用严格的左闭右开区间。SRT 本身是毫秒级格式，任意 ticks 无法无损导出为 SRT。字幕排版与屏幕显示归属于内容层；音频事件与编码封装将在后续章节展开。

### 预计算字幕排版 (Prepared Caption Layout)

`CaptionStyle` 接收对角顶点定义的防遮挡安全矩形 Rect、文本颜色与 f64 尺寸。`layout(track, canvas_size, &fonts)` 强制要求传入显式冻结的 `FontLibrary`，并预先计算出所有段落/矢量节点。`CaptionLayout::node(Time)` 挑选有效的左闭右开线索，自底向上智能堆叠重叠字幕，避免逐帧重复排版。

关键词高亮匹配为字面精确匹配，遵循最长匹配优先、插入顺序稳定与字素簇安全规则；Tab 制表符自动展开为 4 个空格。输入文本、线索条数、Span 切片、关键词与行数均设有严格的配额限制。超出安全区或单屏字幕总高度超额会直接报错，绝不静默将溢出文字裁切掩盖。可通过审查 `cues` 与 `safe_area` 来检验测量准备结果。

## 资源存储与诊断机制

共享 `ResourceStore` 必须显式声明。不同的 Assets 所有者默认拥有隔离的存储空间。共享存储并不意味着不同根目录、不同源文件或不同字体快照的所有者可以任意混用缓存。

`Assets` 的克隆、快照、预加载与移除遵循截然不同的契约。注销某一资产不会强制销毁仍被外部句柄或画面持有的内存数据。逻辑计费字节不代表系统物理 RSS 内存。

场景中的 `DrawContext` 共享一份不可变的 `Evaluation`，其中包含时间、画质等级、诊断器、字体、资源与渲染缓存（单个可选的 `RenderCache`；排版布局缓存使用其 paragraph 分区）。跨线程或先前捕获的句柄使用 `get_with` 传递显式诊断收集器；系统不存在可供隐式依赖的单帧全局上下文。

## 常见踩坑点

- 空的 FontLibrary 绝不会自动具备操作系统字体回退能力。在渲染文本前必须首先注册项目字体。
- 仅加载英文字体在运行时会导致中文字形缺失方块，即使代码编译完全成功。
- 切勿混淆当前工作目录路径与资产根目录路径。请始终通过代码显式配置资产根目录，而非依赖程序碰巧在哪个目录下启动。
- 应当尽量复用资源所有者、句柄与已准备的内容对象，避免在每一帧内重复解码静态图像或重建编译器环境。
- `allow_missing_assets` 绝非正常的生产级配置。占位图与诊断警告并不代表资源已经正确加载。
- `mark_invalid_geometry` 是仅用于视口预览的形状容错策略：非法形状会被绘制为洋红色占位标记。视频导出与 CLI 命令行始终保持严格报错。

## 后续章节

下一章：[07 粒子系统](07-particles.md)。若暂不涉及粒子，可直接跳至 [08 Typst 与数学公式](08-formula-typst.md) 或 [09 渲染与合成](09-rendering.md)。
底层契约参考：[内容准备契约](../../zh-CN/design/content.md)、[共享矢量内容](../../zh-CN/design/content.md#svg-与矢量树-vector-trees)、[字体底层实现](../../../crates/motion-content/src/font.rs)。
范例短片：[Room to Move](../../../gallery/src/demos/type_in_motion.rs)、[The Long Way Home](../../../gallery/src/demos/ink_journey.rs)。
