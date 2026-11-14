# 16 声明式 JSON 文档

## 本章学习目标

使用纯 JSON 数据而非 Rust 原生闭包来完整描述一部短片，将其加载编译为 `Timeline`，通过 `Project` 或命令行进行渲染，并在 Rust 代码中以强类型 Builder 方式构建相同的文档。
请启用 `declarative` 特性（默认已开启）。文本、图像与 SVG 元素分别需要开启 `text`、`image` 与 `svg` 特性。底层格式契约详见[声明式文档设计契约](../design/documents.md)。

## 最小文档示例

将此文档保存在 `examples/documents/` 目录下，以便其相对路径 `$schema` 能够解析到仓库中的[标准 Schema 规范](../../../schemas/motion.schema.json)。

```json
{
  "$schema": "../../schemas/motion.schema.json",
  "version": 1,
  "size": [640, 360],
  "fps": 30,
  "background": "#0c1311",
  "tracks": [{
    "items": [
      {"scene": {"duration": 3, "content": {
        "circle": {"center": [0, 0], "radius": 28},
        "fill": "#14a082",
        "position": {"tween": {"from": [120, 180], "to": [520, 180], "duration": 3,
                               "easing": "cubic_in_out"}}
      }}},
      {"transition": {"effect": "cross_fade", "duration": 0.5}},
      {"scene": {"duration": 2, "content": {
        "text": {"content": "Hello", "family": "Noto Sans", "size": 48},
        "fill": "white",
        "position": [320, 180]
      }}}
    ]
  }],
  "fonts": ["NotoSans-Regular.ttf"]
}
```

`version` 字段为必填项且取值必须为 `1`。时间单位为秒，旋转角为顺时针度数。每个元素至多拥有一个内容键（`children`、`rect`、`circle`、`ellipse`、`line`、`arc`、`polygon`、`polyline`、`regular_polygon`、`star`、`ring`、`arrow`、`spline`、`path`、`text`、`image` 或 `svg`），外加可选的样式、变换与滤镜键。[`examples/documents/intro.json`](../../../examples/documents/intro.json) 是一个完全基于纯几何的完整短片示例。

## 渲染声明式文档

`load` 读取磁盘文件并将相对字体、图片、SVG 与音频路径基于该文档所在目录解析。调用 `render_options` 以便将文档声明的 Look 风格应用到最终输出上：

```rust,no_run
use motion::prelude::*;

fn main() -> motion::Result {
    let film = motion::declarative::load("examples/documents/intro.json")?;
    let options = film.render_options(RenderOptions::default());
    render_still(film.timeline(), 1.seconds().after_origin(), &options)?.save_png("document.png")
}
```

使用 `Project::document` 可将 JSON 文档直接注册到项目命令行中。任何 `Project` 命令行程序同样支持直接传入 `.json` 路径替代 Timeline 具名参数，`schema` 子命令输出 JSON Schema 用于编辑器智能补全：

```sh
cargo run -p motion-cli -- still examples/documents/intro.json --time 1.5
cargo run -p motion-cli -- schema > schemas/motion.schema.json
```

```rust,no_run
use motion::prelude::*;

fn main() -> std::process::ExitCode {
    Project::new("films")
        .document("intro", "examples/documents/intro.json")
        .run()
}
```

## 动画属性 (Animated Properties)

任何标量数值、坐标点或颜色属性均既可以接收固定常量，也可以接收单个运动对象：

| 运动类型 | JSON 表达形式 | 说明与选项 |
| --- | --- | --- |
| Tween (补间) | `{"tween": {"from": 0, "to": 1, "duration": 2, "easing": "back_out"}}` | 支持可选的 `delay`、`repeat`、`ping_pong` |
| Keyframes (关键帧) | `{"keyframes": {"keys": [{"time": 0, "value": 0}, {"time": 1, "value": 5, "easing": "sine_out"}]}}` | 时刻必须严格单调递增；缓动属性归属于抵达的关键帧 |
| Oscillate (振荡) | `{"oscillate": {"frequency": 2, "amplitude": 0.5, "offset": 1}}` | 仅限数值型；`waveform` 支持 `sine`、`triangle`、`square` 或 `sawtooth` |
| Noise (噪声) | `{"noise": {"seed": 7, "frequency": 1, "amplitude": 4}}` | 仅限数值型；带随机种子且完全确定性 |

无参数的标准缓动可以直接使用字符串，如 `"cubic_in_out"`；带参数的缓动使用对象表达，如 `{"steps": 4}` 或 `{"cubic_bezier": {"x1": 0.2, "y1": 0, "x2": 0, "y2": 1}}`。

系统还提供物理弹簧、路径跟随分量与时间组合子：

```json
{"spring": {"from": 0, "to": 1, "stiffness": 121, "damping": 9}}
```

```json
{"clock": {"source": {"spring": {"from": 0, "to": 1}}, "scale": 2, "offset": -0.2}}
```

```json
{"map": {"source": {"tween": {"from": 0, "to": 1, "duration": 1}}, "expr": "t * t"}}
```

`clock` 负责缩放平移嵌套动画的时间轴，而 `map` 则将 `t` 视为其采样的输入数值。`components` 将独立的标量动画组合为坐标点或矩形；`sum` 按顺序累加多个标量源。`follow_path` 从 SVG 路径数据按弧长输出 `x`、`y` 或顺时针度数的切线角度 `angle`。

若需实现动态数字跳动读数，使用 `text.number` 替代固定文本：

```json
{"text": {"number": {"value": {"expr": "t"}, "precision": 2, "prefix": "t ", "suffix": " s"}, "size": 14}, "fill": "white"}
```

详细限制请参阅[数值文本读数规范](../design/documents.md#数值文本读数-numeric-text-readouts)。各个独立页面作为 Scene 依次排列在 Track 轨道中；每个场景拥有各自的局部时钟，所有场景在编译期均会被严格校验。

## 样式、空间变换与效果

| 配置键 | 含义与说明 |
| --- | --- |
| `fill` | 动画颜色或 `{"linear" \| "radial" \| "sweep": {…, "stops": […]}}` 渐变 |
| `stroke` | 包含 `width`、`paint`、`cap`、`join`、`dash`、`dash_offset`；`join` 支持 `{"miter": {"limit": 4}}`、`"round"` 或 `"bevel"` |
| `trim`, `fill_rule` | 路径绘制弧长区间截取与内部填充绕数规则 |
| `position`, `pivot` | 将轴心（默认几何中心）定位到父级坐标；`pivot` 可为 `"top_left"`、`{"relative": [0.5, 0.5]}` 或 `{"point": [x, y]}` |
| `translate`, `rotate`, `scale`, `scale_xy` | 动画化空间变换；旋转角使用度数 |
| `opacity`, `blend`, `z_index`, `hidden` | 图层合成、混合模式与兄弟绘制层级 |
| `filters` | 有序滤镜链：`blur`、`drop_shadow`、`glow`、`bloom`、`color_adjust`、`cdl`、`halation` |
| `clip`, `mask` | `{"shape": {"rect" \| "circle" \| "path": …}, "mode"}` 与 `{"element", "mode"}` |

轨道项包括 `{"scene": …}`、`{"gap": 秒数}` 以及转场 `{"transition": {"effect", "duration"}}`（转场效果支持 `cross_fade`、`zoom_blur`、`{"slide": {"direction"}}`、`{"push": {"direction"}}`、`{"wipe": {"angle", "feather"}}` 与 `{"iris": {"shape"}}`）。带有 `start` 的场景会精确放置在指定时刻。文档级 `look` 可配置为 `"clean"`、`"cinematic"` 或 `{"film": {"seed": N}}`，`audio` 列表声明音频事件及其 `start`、`offset`、`duration`、`volume`、`fade_in`、`fade_out` 与 `pan`。

## 参数化配置 (Parameters)

在根级 `params` 中声明默认值，并在任何允许出现值的地方通过 `{"$param": "name"}` 进行引用（包括颜色、文本甚至整个子节点数组）。传入覆写参数会完整替换整个默认值，未知参数名会被直接拒绝：

```json
{
  "version": 1, "size": [320, 180], "fps": 30,
  "params": { "radius": 40, "tint": "#14a082" },
  "tracks": [{"items": [{"scene": {"duration": 2, "content": {
    "circle": {"center": [160, 90], "radius": {"$param": "radius"}},
    "fill": {"$param": "tint"}
  }}}]}]
}
```

```rust
use motion::declarative::{CompileOptions, Document};
use serde_json::json;

let text = r##"{"version": 1, "size": [64, 64], "fps": 10,
    "params": {"radius": 8},
    "tracks": [{"items": [{"scene": {"duration": 1, "content": {
        "circle": {"center": [32, 32], "radius": {"$param": "radius"}},
        "fill": "white"}}}]}]}"##;
let document = Document::from_json_with(text, Some(&json!({"radius": 20})))?;
assert_eq!(document.params["radius"], 20);
document.compile(&CompileOptions::default())?;
assert!(Document::from_json_with(text, Some(&json!({"size": 1}))).is_err());
# Ok::<(), motion::Error>(())
```

在命令行中，紧随 `.json` 文件路径后传入 `--params '{"radius": 20}'` 即可完成覆写。加载后的文档对象包含最终生效的值，不再包含 `$param` 占位符。

使用 `$calc` 可以定义在参数覆写合并后仅执行一次计算的数值，而非在每一帧动态求值：

```json
{
  "seconds": 8,
  "fps": 30,
  "pages": 43,
  "page_1_frame": {"$calc": "ceil($seconds * $fps / $pages)"},
  "page_1_start": {"$calc": "$page_1_frame / $fps"}
}
```

这些声明置于根级 `params` 中。覆写 `seconds` 会自动重新计算这两项派生数值，不受书写顺序影响。`$calc` 同样可以直接嵌入到具体数值字段或可复用定义模板中。它遵循与 `expr` 相同的数学语法与函数库，但严禁读取时间 `t` 或元素属性绑定；循环依赖与非有限浮点结果会被报错拦截。运算全程采用 f64 双精度浮点，而未经算术运算的整型参数（如随机种子）保持其原有精度。若参数需要随帧时间动态改变，请使用 `expr`。

## 可复用定义模板 (Defs)

在根级 `defs` 中声明共享的属性值、样式或组件模板。在加载时，`use` 引用会被自动就地展开；同级并列的字段会整体替换模板中的对应字段：

```json
{
  "version": 1, "size": [320, 180], "fps": 30,
  "params": {"tint": "#14a082"},
  "defs": {
    "style": {"fill": {"$param": "tint"}},
    "dot": {"use": "style", "circle": {"center": [0, 0], "radius": 20}}
  },
  "tracks": [{"items": [{"scene": {"duration": 2, "content": {
    "children": [
      {"use": "dot", "id": "left", "position": [100, 90]},
      {"use": "dot", "id": "right", "position": [220, 90], "fill": "white"}
    ]
  }}}]}]
}
```

定义模板支持嵌套引用其他模板，并可包含参数引用。数组与标量定义使用纯 `{"use": "name"}`，不接受同级字段覆盖。例如覆写 `circle` 必须提供完整的圆形描述。循环引用与未声明的模板名会在引用路径处报错。加载完成的文档直接序列化展开后的完整数值，不包含 `defs`；元素 ID 严格保持创作者所写的内容。

## 表达式与属性绑定

数值、坐标点或矩形可以写为 `{"expr": "..."}`；坐标点支持分量表达式。`t` 代表场景局部秒数，`$name` 读取数值型参数。只要为某个元素指定了 `id`，同场景内的其他元素便可通过 `@id.property` 语法读取其属性值：

```json
{"id": "planet", "circle": {"center": [0, 0], "radius": 10}, "fill": "#6ad6b8",
 "position": {"expr": ["320 + $radius * cos(t)", "180 + $radius * sin(t)"]}},
{"circle": {"center": [0, 0], "radius": 4}, "fill": "white",
 "position": {"expr": ["@planet.position.x + 28 * cos(t * 4)", "@planet.position.y + 28 * sin(t * 4)"]}}
```

支持丰富的内置数学函数；比较运算符返回 1 或 0。属性绑定读取的是目标元素自身的属性计算值，不包含父级空间变换。循环依赖、未知 ID 以及非数值路径均属于错误。[`examples/documents/orbit.json`](../../../examples/documents/orbit.json) 综合展示了这两项能力。

## 更多富媒体内容类型

| 内容关键字 | 所需 Feature | 渲染画面说明 |
| --- | --- | --- |
| `morph` | 无 | `{"from": "M…", "to": "M…", "progress": …}`，按元素的 fill 与 stroke 绘制 |
| `svg_morph` | `svg` | 两个 SVG 文件按 ID、坐标与形状自动匹配变形 |
| `particles` | `particles` | 包含 `seed`、`emitters`、`modifiers`、`style`、`blend` |
| `trajectory` | `particles` | 解析闭式 `ballistic`、`orbit` 或 `noise_drift` 粒子爆发 |
| `ruler`, `callout`, `line_chart` | `diagram` | 坐标轴、引出标注与统计图表；图表自动从 `bounds` 推导坐标轴范围 |
| `captions` | `captions` | 内联 `cues` 或引用外部 `srt` 文件，结合文档中的 `fonts` 进行排版 |
| `formula` | `formula` | Typst 数学公式排版，支持 `write` 展开与 `morph_to` 变形 |

粒子颜色采用 sRGB 输入。标尺、引出标注与图表文本通过渲染字体环境绘制，因此当文档声明了 `fonts` 时，请通过 `CompiledDocument::render_options`（或 `Project` 与 CLI）进行渲染。

## 富文本与逐字动效

`text` 接受 `content` 纯文本或 `spans` 切片列表，每个切片支持自定义 `family`、`size`、`weight`、`italic` 与 `fill`。`split` 支持按字符（`characters`）、单词（`words`）或行（`lines`）进行切分；`visible_parts` 控制前几个可见切片，因此 `{"expr": "floor(t * 12)"}` 可轻松实现打字机效果。`parts` 支持为每个切片配置 `delay` 阶梯延迟秒数，并驱动 `offset`、`scale`、`rotate` 与 `opacity`：

```json
{"text": {"content": "Staggered", "family": "Noto Sans", "size": 40,
          "split": "characters",
          "parts": {"delay": 0.08, "offset": {"tween": {"from": [0, 16], "to": [0, 0], "duration": 0.4}},
                    "opacity": {"tween": {"from": 0, "to": 1, "duration": 0.4}}}},
 "fill": "white", "position": [320, 180]}
```

## 音频声明

```json
{
  "buses": [{"name": "music", "gain_db": -6, "effects": [{"reverb": {"room": 0.3}}]}],
  "master": {"limiter": {"ceiling_db": -1}},
  "audio": [
    {"id": "voice", "source": "voice.wav", "pan": {"tween": {"from": -0.5, "to": 0.5, "duration": 4}}},
    {"source": "music.wav", "bus": "music", "volume": {"expr": "1 - t / 60"},
     "ducked_by": [{"trigger": {"clip": "voice"}, "depth_db": 18}]}
  ]
}
```

动画化的 `volume`、`pan` 与 `spatial.position` 在以片段起点为基准的相对时间轴上采样。效果器（需要 `audio` 特性）包含 `eq`（支持自动化频率与增益）、`compressor`、`gate`、`de_esser`、`delay` 与 `reverb`。片段可通过 `id` 被先前的片段动态压低音量，总线同样支持闪避。

## 错误诊断、预检与 Studio 热重载

从文件加载的文档能精确报告出错位置并智能推荐拼写修复：

```text
tracks[0].items[0].scene.content.fil: unknown field `fil`; 45 names are valid here. (line 4, column 4) Did you mean `fill`?
```

`validate` 子命令在不渲染任何画面的情况下校验文档：它构建时间轴、检查所有外部引用的文件是否存在，并对首帧、中间帧与尾帧执行 preflight 试飞检查：

```sh
cargo run -p motion-cli -- validate examples/documents/intro.json
cargo run -p motion-cli -- validate orbit.json --params '{"radius": 40}'
```

在 Rust 中，`Project::check` 返回 `ValidationReport`。为了在编辑时获得实时预览，在 Studio 中打开文档；保存该文档或其引用的文件，场景会即时就地重载并保留当前播放指针位置。若重载出错，界面保持显示上一版本的有效场景，并在顶部导航栏高亮显示错误信息：

```sh
cargo run --release -p motion-cli -- studio examples/documents/orbit.json
```

## 在 Rust 代码中构建文档

JSON 数据结构同样是一套完整的强类型 Builder API。文档支持无损往返序列化：写出并重新读入能够保证数值严格完全一致。

```rust
use motion::declarative::{
    Animated, CompileOptions, Document, EasingSpec, Element, SceneSpec, TrackSpec, TweenSpec,
};
use motion::prelude::*;

let document = Document::new([320.0, 180.0], 30).track(TrackSpec::new().scene(
    SceneSpec::new(
        2.0,
        Element::circle([160.0, 90.0], 24.0)
            .fill(Color::WHITE)
            .scale(TweenSpec::new(0.0, 1.0, 1.0).easing(EasingSpec::BackOut { overshoot: 1.7 }))
            .opacity(Animated::tween(0.0, 1.0, 0.5)),
    ),
));
let json = document.to_json()?;
assert_eq!(Document::from_json(&json)?, document);
let film = document.compile(&CompileOptions::default())?;
assert_eq!(film.timeline().frame_count(), 60);
# Ok::<(), motion::Error>(())
```

## 诊断体系与错误捕获

解析阶段报告首个结构性错误的 JSON 路径。编译阶段校验所有属性并集中汇总所有故障，每个错误作为 `ValidationError` 呈现，其 `path` 精确对应文档的树形层级，例如 `tracks[0].items[0].scene.content.children[1].circle.radius`。

```rust
use motion::declarative::Document;
use motion::prelude::*;

let error = Document::from_json(
    r#"{"version": 1, "size": [64, 64], "fps": 24, "tracks": [{"items": [
        {"scene": {"duration": 1, "content": {"circle": {"center": [8, 8], "radius": 4}}}}]}]}"#,
)?
.compile(&Default::default())
.unwrap_err();
let diagnostic = error.errors()[0].downcast_ref::<ValidationError>().unwrap();
assert_eq!(diagnostic.path, "tracks[0].items[0].scene.content");
assert_eq!(diagnostic.field, "fill");
# Ok::<(), motion::Error>(())
```

## 常见踩坑点

- 几何图形若未声明 `fill` 填充或 `stroke` 描边会被当作语法错误，系统不会默认将其视作透明不可见形状。
- 关键帧上的 `easing` 决定的是**抵达**该帧的曲线段；首个关键帧上的缓动设置会被直接忽略。
- 包含 `..` 的路径会被直接拒绝；相对路径仅在通过 `load` 加载或显式配置了基准目录时相对于文档目录解析。
- 未开启对应特性的内容类型（`text`、`image`、`svg`、`particles`、`diagram`、`captions`、`formula`、音频效果器）虽然能正常通过语法解析与序列化，但在编译期会抛出特性未启用的错误。
- 属性绑定的作用域严格限定在同一个场景内部，且读取的是目标元素自身的属性计算值，而非其全局世界变换。
- `$param` 替换发生在类型校验之前：参数若提供了错误的类型，会在具体的引用路径处报错。
- JSON Schema 负责结构性校验；复杂的数值合法区间与时间轴时序由编译流水线深度保障。

## 后续章节

阅读[声明式文档设计契约](../design/documents.md)了解深入的配额与设计细节，阅读[命令行 CLI 文档](../cli.md#声明式文档)了解 `schema` 命令用法。
