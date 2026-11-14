# 项目命令行 CLI

在可执行文件中向 `Project` 注册 Timeline 并调用 `run()` 即可构建一个命令行程序。相同的 clap 命令行语法服务于 `examples/motion.rs`、Gallery 以及任何短片二进制包。本页面是 [`project.rs`](../../crates/motion/src/project.rs)、[`args.rs`](../../crates/motion/src/project/args.rs) 和 [`cli.rs`](../../crates/motion/src/project/cli.rs) 中实现的子命令、命令行参数与错误处理行为的参考手册。专业术语遵循[设计契约](design/README.md)：*still*（静帧）代表渲染的单个画面帧，*film*（短片）代表导出的视频文件。

## 将可执行文件转换为 CLI

```rust,no_run
use motion::prelude::*;

fn main() -> std::process::ExitCode {
    Project::new("motion")
        .timeline("motion", || {
            Timeline::new(Size::new(640.0, 360.0), fps!(30)).track(|track| {
                track.scene(3.seconds(), |context| -> motion::Result<_> {
                    let x = context.sample(&Tween::new(80.0, 560.0, 3.seconds()));
                    Ok(Circle::new(Point::new(x, 180.0), 32.0)?.fill(Color::rgb(20, 160, 130)))
                })
            })
        })
        .run()
}
```

以上是 [`examples/motion.rs`](../../examples/motion.rs)。`Project::run` 将错误以 `Display` 格式输出到 stderr 并返回 `ExitCode::FAILURE`，成功则返回 `SUCCESS`。`try_run` 返回 `ProjectOutcome`（`Help`、`Listed`、开启 `declarative` 特性时的 `Schema`、`Validated`、`Still`、`Exported`、`StudioClosed`），`cli(args)` 解析显式参数列表（不含程序名）；`print_outcome` 打印执行结果。Gallery（[`gallery/src/main.rs`](../../gallery/src/main.rs)）将 `ProjectCommand` 嵌入到其专有的 clap 解析器中（增加了 `--font` 以及 `--timeline` 的别名 `--demo`），并调用 `execute` 执行。

## 注册规则

- `timeline(name, build)` 注册延迟加载工厂；`timeline_with::<P>` 与 `try_timeline_with::<P>` 注册带类型参数的工厂。注册和 `list` 过程从不调用构造闭包，也不访问字体、GPU 设备或编码器。
- 开启 `declarative` 特性时，`document(name, path)` 注册一个 JSON 文档，在每次构建时读取并编译；参见[声明式文档](#声明式文档)。
- 名称为 1..=128 字节的 ASCII 字母、数字、`.`、`_` 或 `-`，唯一且最多支持 256 项。项目名称必须为 1..=256 字节。
- 无假定失败的 builder 方法所产生的注册错误会被延迟收集，并在首次调用 `cli`/`build` 时作为一个聚合的 `ErrorReport` 在解析任何参数之前统一报错。
- `Project::render_options` 提供共享字体/资源及默认选项。CLI 始终在应用 `--backend` 参数之前将后端首选项覆盖为引擎默认值（启用 `gpu` 时为 GPU）。

## 子命令

| 命令 | 行为说明 | 所需特性 |
| --- | --- | --- |
| `list` | 打印格式化 JSON 数组形式的 `{name, defaults}`；不接受额外选项 | 无 |
| `still NAME` | 渲染指定的一帧并输出为 PNG | 无 |
| `export NAME` | 编码导出短片（或 PNG 序列/GIF） | H.264/H.265/VP9/ProRes 需要 `ffmpeg` |
| `studio [NAME]` | 为指定 Timeline 打开 [Studio](studio.md) 工作台 | `studio` |
| `validate NAME` | 编译并校验短片而不进行渲染；仅接受 `--params`/`--params-file` | 无 |
| `schema` | 输出声明式文档的 JSON Schema；不接受任何选项 | `declarative` |

无参数调用、`--help`、`-h` 或 `help` 子命令会打印使用帮助并以状态码 0 退出（不加载任何内容）；`COMMAND --help` 打印对应子命令的选项。`still` 和 `export` 需要位置参数 `NAME`（命令后的第一个非 `-` 开头参数）。`studio` 接受 `NAME` 或 `--timeline NAME`（二者互斥）；`--timeline` 仅存在于 `studio` 子命令。若未启用 `studio` 特性，`studio` 会报错指出缺失特性（在拒绝无效选项之后）。若未启用 `ffmpeg` 特性，FFmpeg 编码器会报错“export requires feature ffmpeg”；PNG 序列和 GIF 编码器是独立的实现，不依赖 FFmpeg。默认特性集会同时开启这两者（`crates/motion/Cargo.toml`）。

## 命令行选项标志

除 `--overwrite`、`--alpha` 和 `--allow-software-gpu` 为布尔开关外，其余选项均需在其后跟随取值。语法错误来自 clap：重复给出选项、未知选项、缺失或无效参数以及冲突选项（如 `--time` 与 `--frame`，`--params` 与 `--params-file`）都会输出帮助用法并失败。未被某个子命令定义的选项（如 `still` 上的 `--range`）会被判定为未知。拥有自定义选项的程序可将 `ProjectCommand` 嵌入到自身的 clap 解析器中并调用 `Project::execute`（正如 Gallery 所做的那样）。

| 选项 | 适用命令 | 含义说明 |
| --- | --- | --- |
| `--params JSON` | 全部 | 行内传入 JSON 参数；超过 1 MiB 将被拒绝 |
| `--params-file PATH` | 全部 | 从不超过 1 MiB 的常规文件中读取参数 |
| `--size WIDTHxHEIGHT` | 全部 | 输出像素分辨率（两个以 `x` 分隔的 `u32`） |
| `--backend cpu\|gpu` | 全部 | 渲染后端首选项 |
| `--allow-software-gpu` | 全部 | 允许 `RenderOptions::backend` 中的 `Gpu` 使用软渲染适配器；不在帮助中列出 |
| `--output PATH` | still, export | 目标输出路径；默认为 `NAME.png` 或 `NAME.EXT` |
| `--overwrite` | still, export | 覆盖已存在的目标文件 |
| `--time T` | still | 截取帧的时间；默认为 `0` |
| `--frame N` | still | 帧索引（非负 `i64`），按 Timeline 帧率换算 |
| `--range A:B` 或 `A..B` | export | 左闭右开时间区间；要求 `A < B` |
| `--codec NAME` | export | 编码器名称；默认为 `h264` |
| `--quality N` | export | 质量等级参数 `u8`；默认为 18 |
| `--alpha` | export | 请求保留透明通道（仅限 VP9） |
| `--timeline NAME` | studio | 替代位置参数的 Timeline 选择选项 |

`--params` 与 `--params-file` 互斥。同时指定 `--time` 与 `--frame` 会报错；`export` 拒绝 `--time` 与 `--frame`（请使用 `--range`）；`still` 拒绝 `--range`、`--codec`、`--quality` 与 `--alpha`。`studio` 仅接受参数、`--size`、`--backend`、`--timeline` 与 `--allow-software-gpu`；任何 output/time/range/overwrite/frame/codec/quality/alpha 标志均会视为错误。

### 时间解析语法

`parse_time` 接受带可选单位后缀的十进制数：`1.5`、`1.5s`（秒）或 `1500ms`。取值必须为有限数值且在 `0..=86400` 秒范围之内。`--range` 按首个 `..` 或首个 `:` 进行切分，因此 `2s..6s`、`1:3` 和 `500ms..2s` 均可正常解析。

### 编解码器名称

`parse_codec` 将输入名称映射为 `Codec` 枚举；若对不支持质量设定的编码器传入 `--quality` 将被拒绝。

| 名称 | 编解码枚举 | 默认扩展名 | 质量调节 | Alpha 透明通道 |
| --- | --- | --- | --- | --- |
| `h264` | `Codec::H264` (Medium 预设) | `.mp4` | 支持 | 拒绝 |
| `h265`, `hevc` | `Codec::H265` (Medium 预设, 8-bit) | `.mp4` | 支持 | 拒绝 |
| `vp9` | `Codec::Vp9` | `.webm` | 支持 | `--alpha` 支持 |
| `prores`, `prores4444` | `Codec::ProRes4444` | `.mov` | 拒绝 | 默认支持，无需标志 |
| `png`, `png-sequence` | `Codec::PngSequence` | `.frames`（输出目录） | 拒绝 | 默认支持，无需标志 |
| `gif` | `Codec::Gif` (256 色, Timeline 帧率) | `.gif` | 拒绝 | 默认支持，无需标志 |

其他名称会失败报错 `Unknown codec`。构建的编解码器还会通过 `Codec::validate` 验证。帮助文档中仅列出 `h264|h265|vp9|prores4444|png|gif`；上述别名均可被接受，但未在帮助中显式列出。

## 输出路径与文件覆盖

- 静帧输出默认在当前目录下生成 `NAME.png`，导出视频默认生成 `NAME.EXT`（参见上表）。对于文档路径，`NAME` 为文件主名（stem）。路径不会进行其他额外规范化。
- **输出目录必须已存在。** 两个命令均在目标文件同级创建临时文件过渡暂存；若目录缺失会报错失败：`No such file or directory ... .motion-still-*.png`（或 `.motion-video-*.mp4`）。CLI 绝不会隐式创建目录。
- 未提供 `--overwrite` 时，若目标文件已存在则操作失败：`still` 会预先检查并报告 `Still destination exists`；`export` 报告 `Export destination exists`。传入该选项后，新文件生成完毕后会原子替换已有文件。
- 两个命令均采用暂存提交机制；参见[可靠性](design/reliability.md)与[渲染](design/rendering.md)中的事务规则。
- 执行成功会在 stdout 上输出目标路径（静帧路径或 `ExportReport::path`）。编码器底层库可能会向 stderr 打印日志。
- 静帧使用项目的渲染选项进行渲染（除非项目显式设置背景色，否则默认为透明）。导出视频在未指定背景色时默认合成在黑色背景上。

## 参数化配置

参数在构建闭包执行之前先进行准入校验（[`project.rs`](../../crates/motion/src/project.rs)）：

| 校验规则 | 失败原因 |
| --- | --- |
| 常规 `timeline`/`try_timeline` 不接受参数 | `Timeline does not accept parameters` |
| `P::default()` 序列化为 JSON 对象不得超过 1 MiB | 注册错误 |
| 传入的 JSON 必须为对象结构 | `Parameters must be a JSON object` |
| 传入的键必须存在于序列化的默认值中 | `Unknown parameter: KEY` |
| 合并后的值必须能够反序列化为 `P` | `invalid parameters:` 附带 serde 错误信息 |
| 序列化参数总长超过 1 MiB | `Parameters exceed 1 MiB` |

参数合并为**顶层整键替换**：用户传入的每个键都会完整替换默认值，包括嵌套对象。`list` 将 `defaults` 作为自省界面展示（非参数化 Timeline 显示为 `null`）。构建完成的 Timeline 在使用前会先经校验。默认值与反序列化逻辑应当保持确定性且无副作用。Gallery 中的 `DemoParameters`（包含 `seconds`、`seed`、`count`）便是一个典型范例。

## 声明式文档

启用 `declarative` 特性（facade 默认开启）后，在未注册对应同名 Timeline 的前提下，`still`、`export` 和 `studio` 可直接接受以 `.json` 结尾的文件路径。文档在执行命令时被读取并编译，其相对资源路径相对于文档所在目录解析，其 Look 的显示阶和字体会自动应用至渲染选项。`--params` 会覆盖文档中在 `params` 字段声明的参数；未知参数名会报错，且 `list` 不显示默认值（因为 listing 操作绝不主动读取磁盘文件）。`schema` 输出该格式的 JSON Schema 规范。[`crates/motion-cli`](../../crates/motion-cli) 中的 `motion` 程序使用 clap 解析参数并直接支持文档路径输入（包含 `validate`、`still`、`export`、`studio`、`schema`；每个子命令均支持 `--help`）：

```sh
cargo run -p motion-cli -- still examples/documents/intro.json --time 1.5
cargo run -p motion-cli -- schema > schemas/motion.schema.json
```

文档错误会明确指出失败的 JSON 路径、字段，对于已加载的文件还会给出具体的行号和列号，例如 `tracks[0].items[0].scene.content.fill: Geometry requires a fill or a stroke (line 4, column 5)`。拼写错误的键名、枚举值、参数名、元素 ID、总线名称与表达式函数在存在相近名称时会附带 `Did you mean 'fill'?` 提示。文件格式契约参见[声明式文档设计](design/documents.md)，创作指南参见[教程第 16 章](tutorial/16-declarative.md)。

### Validate 校验子命令

`validate NAME`（底层对应 `Project::check`）构建 Timeline 或文档并对其进行校验，检查文档所引用的所有外部文件是否存在，随后针对首帧、中间帧与尾帧在配置的资源与限制下执行 preflight 试飞检查。此命令完全不渲染、不编码像素，也不开启任何硬件设备，非常适用于 CI 流程与编辑器集成。验证通过会输出单行摘要，如 `intro.json: valid (640x360, 30 fps, 132 frames, 4.4 s, 3 frames preflighted, 1 files)`；任何失败都会输出具体的诊断信息并以状态码 `1` 退出。抽检帧之间的其余帧不会被求值。

### Studio 与文档联动

对 `.json` 文档或通过 `Project::document` 注册的 Timeline 执行 `studio NAME` 会以可热重载模式打开文档：保存该文档或其引用的文件，场景会即时就地重新加载。参见 [Studio 热重载](studio.md#热重载)。

## 退出状态码与错误

执行成功（包括 `--help` 输出）退出状态码为 `0`，任何错误退出状态码均为 `1`；不设更细粒度的错误码。参数错误格式化为 `invalid parameters: MESSAGE`（`Error::Parameters`）；I/O、渲染与编码器错误直接打印各自的 `Display` 文本。以编程方式使用 `try_run` 时，应审查 `Error` 枚举的具体变体而非文本消息（参见[可靠性](design/reliability.md)）。

## Gallery 与 Skill 消费者

Gallery 封装了相同的 CLI 架构（[`gallery/src/lib.rs`](../../gallery/src/lib.rs)）：

- 未提供参数时，若 `studio` 特性开启则运行 `studio`，否则运行 `list`。
- `--font PATH` 会在解析前被提取，且仅加载指定的字体文件；其余命令（除 `list` 外）默认加载系统字体。
- `studio --demo NAME` 选择单个示例（仅限一次且必须为有效名称），与位置参数或 `--timeline` 互斥；不带名称的 `studio` 默认选择目录中的第一个示例。

[`motion-generate` skill](../../.github/skills/motion-generate/SKILL.md) 能够根据自然语言内容描述自动生成独立的 JSON 影片，对其进行校验与自检，随后提供直接的文档路径命令用于 Studio 预览、静帧导出和视频渲染。

自动化脚本可利用 `--size`、`--time`、`--range`、`--output` 和 `--overwrite` 驱动短片二进制的 `still` 与 `export` 命令，并依赖状态码 `0` 判断结果。[`visual_check.rs`](../../crates/motion/src/visual_check.rs) 中的 `VisualCheckPolicy` 是可选的注册元数据：通过 `ProjectBuilder::visual_check` 挂载，通过 `Project::visual_check` 读取。它由 crate 根模块导出但未包含在 prelude 中，CLI、渲染与导出流水线从不读取它。

## 相关参考

- [Studio 用户指南](studio.md)
- [导出教程](tutorial/11-export.md)
- [渲染与导出契约](design/rendering.md)
- [系统可靠性](design/reliability.md)
- [开发维护检查](development.md)
- 源码实现：[`cli.rs`](../../crates/motion/src/project/cli.rs)、[`args.rs`](../../crates/motion/src/project/args.rs)
