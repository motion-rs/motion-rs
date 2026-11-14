# 资源与字体管理契约

本页面记录了资源字节的所有权归属、生命周期边界以及缓存复用条件。除非创作者明确在代码中请求，否则系统绝不会从磁盘读取任何文件、扫描操作系统字体或在显卡设备上解码数据。上手指南请参考[内容系统教程](../tutorial/06-content.md)；特定内容的设计决策见[内容系统契约](content.md)，准入限制见[可靠性规范](reliability.md)与[渲染设计](rendering.md#资源与内容)。

## 所有权模型

| 对象 | 拥有内容 | 生命周期 |
| --- | --- | --- |
| `Assets` | 带类型的条目表、`LoadContext`（绝对根路径、可选 `FontEnvironment`、命名空间） | 克隆实例共享已注册项；调用 `snapshot()` 会冻结已解析条目并隔离后续修改 |
| `ResourceStore` | 逻辑字节计数器与弱引用条目索引 | 由克隆实例共享；新创建的 `Assets` 拥有自己的存储，除非使用 `with_store` |
| `Handle<A>` | 单个共享条目：数据源、粘性结果、计费字节数 | 持有至最后一个 handle 或所有者 drop；drop 时从存储中释放计费 |
| 解码后的 `Arc<A>` | 不可变的解码值 | 生命周期可超过所有者与句柄；此后不再计入存储统计 |
| `FontLibrary` | 字体数据库、塑形引擎、字形/布局缓存、使用情况日志 | 克隆的字体库共享同一份冻结快照 |
| `RenderCache` | 中间计算缓存：拥有 7 个独立有界的分区 | 由克隆实例、渲染会话与导出工作线程共享，直至显式 `clear` |
| `GpuContext` | 显卡设备端不可变图像/Mipmap/Atlas 缓冲区与已编码对比数据 | 由上下文克隆实例共享；有界 LRU 驻留独立于单帧租约 |
| `SceneCache` | 纯创作期计算的 LRU 缓存，以调用点位置、类型与键为索引 | 每个缓存独立拥有；条目遵循 LRU 逐出规则 |

`ResourceStore` 仅持有 `Weak` 弱引用：它本身绝不保持解码数据常驻内存。`Assets::remove` 会注销一个数据源但不会使现存句柄失效，保留的快照或句柄在其 drop 之前持续计费。对应源码实现为 [`asset.rs`](../../../crates/motion-core/src/asset.rs)、[`handle.rs`](../../../crates/motion-core/src/asset/handle.rs) 与 [`store.rs`](../../../crates/motion-core/src/asset/store.rs)。

## 显式 I/O 策略

- `Assets::new(root)` 捕获绝对根路径（`std::path::absolute`），不执行任何磁盘读取或解码。`Assets::default()` 根路径为空。
- 在空根路径下使用相对路径 `Source::File` 会直接返回失败句柄（提示“require an explicit project root”）；空所有者绝不拥有隐式访问当前工作目录的权限。包含 `..` 的相对路径会被直接拒绝；绝对路径原样接受。
- `Assets::load`、`AssetsExt::image`、`Image::open` 与 `Svg::open` 仅保留数据源路径。首次调用 `Handle::get`（或编译实际绘制该资源的场景时）才会真正读取并解码。
- 文件读取要求文件必须为不超过 64 MiB 的常规文件（`Asset::decode` 仅接收原始字节切片，绝不接收文件路径）。`Source::Bytes` 的键必须为 1..=1024 字节且不含控制字符；字节内容本身参与缓存标识计算，因此用不同内容复用相同名称绝不会错误返回陈旧资产。
- `FontLibrary::new()` 初始为空，绝不触碰文件系统。仅当显式调用 `load_system_fonts` 与 `load_system_family` 时系统字体才可用；它们先索引元数据并在首次使用时加载字体文件。追求确定性的项目应调用 `load_file`、`load_dir` 或 `load_data` 固化字体字节。
- SVG 拒绝非 `data:` 协议的外部 `href` 链接、脚本、`foreignObject` 与动画元素；SVG 文本排版需要非空的 `FontLibrary`。
- `FormulaCompiler::new` 与 `TypstCompiler::new` 仅使用内嵌字体，不执行系统字体探测，遇到 import 导入、外部文件与外部包时直接报错。

## 字体库管理

`FontLibrary` 遵循写时复制语义：任何修改操作（`load_*`、`default_family`）均会产生新的 `snapshot_id`，并重置文字塑形引擎、排版缓存与使用日志。克隆实例共享上一份快照。`load_data` 对每个字体库最多准入 4096 个字体字面（Face）与 256 MiB 源字节，且操作具备原子性：损坏的负载不会产生任何部分修改。`load_dir` 对条目进行确定性排序，不追踪符号链接目录，最多遍历 4096 个条目。`load_file` 接受最大 256 MiB 的常规文件，而延迟加载的 `FontFace` 资产（`AssetsExt::font`）受限于 `MAX_FONT_FACE_BYTES`（32 MiB）。系统字体元数据索引上限为 65,536 个 Face。首个加载成功的字体自动成为默认的 sans-serif、serif 与 monospace 字体家族。

Core 核心包绝不初始化字体引擎。它仅传递一个不透明的 `FontEnvironment`（`Arc<dyn Any + Send + Sync>` 附带 `snapshot_id`）；仅有 `motion-content` 可以将其向下转型。`Assets::with_fonts` 会派生出拥有空条目表的新视图，确保此前解析的 SVG 不会被错误地复用于另一套字体环境中。通过 `Text::fonts` 显式绑定的 `Text` 会覆盖 `DrawContext` 中的环境；文本排版必须具备其中之一。参见 [`font.rs`](../../../crates/motion-content/src/font.rs)。

## 缓存身份识别规则

| 缓存类型 | 缓存键（Key）包含的要素 |
| --- | --- |
| `Assets` 资产条目 | 资产类型、绝对根路径、字体 `snapshot_id`、快照命名空间、`Source`（含完整字节） |
| 段落排版布局 | 文本内容、排版样式、排版宽度，加上字体 `snapshot_id` |
| 字形矢量轮廓 | 字体 `snapshot_id`、Face ID、字重 Weight、Glyph ID |
| 数学公式 | 源码、字号、字体家族、填充画刷、调用方 `revision`、`FORMULA_BACKEND_VERSION`、编译器 `environment` |
| `SceneCache` | 调用点位置源码信息、键类型、哈希后的键全等性校验 |
| GPU 不可变资源 | 编码后的 Mipmap/Atlas 元数据及每个纹素精确的 `f32` 内存比特；哈希匹配后要求逐字节完全相等 |

任何可能改变解码或排版结果的因素都必须纳入缓存键中。纯场景计算值应在创作键中包含资源版本修订号。`SceneCache` 构建闭包可能会被执行多次（并发未命中、缓存逐出、`clear`、禁用缓存等），因此闭包必须保持为纯函数。共享同一个 `ResourceStore` 绝不会让拥有不同根路径、不同数据源或不同字体快照的所有者错误复用彼此的条目。

GPU 显存驻留不包含逐次绘制的变换矩阵、覆盖率或实例记录。完全相同的不可变数据字可在不同帧以及不同解码源之间跨帧复用，无需绑定特定资产指针。针对完全相同哈希的资源最多保留 4 个；后续冲突会直接绕过驻留，而非错误混淆内容或引发无界的逐字节比对。命中缓存、缓存逐出、清空缓存与禁用驻留机制绝不改变资源语义或渲染像素。

## 预算配额与内存计量

所有字节限制均为**逻辑有效负载**估算，而非系统进程的物理 RSS 内存：内存分配器元数据、解析器暂存空间以及外部 `Arc` 强引用均被排除在外。

| 作用范围 | 默认配额 | 说明与约束 |
| --- | --- | --- |
| 编码后资产文件输入 | 64 MiB | 在文件元数据、原始字节及读取完毕后分别校验 |
| `ResourceStore` 存储 | 256 MiB, 4096 条目 | `with_limits(ResourceLimits { max_bytes, max_entries })` 要求正字节数与 1..=4096 条目；独立于中间计算缓存的 `CacheLimits`；至少计入编码长度 |
| 静态图像解码 | 单边最大 16384 px, 16,777,216 像素 | 仅限静态 PNG/JPEG/WebP；解码后存储为带 Mipmap 的线性预乘 RGBA16F |
| SVG 源码文件 | 8 MiB, XML 深度 64, 100,000 节点 | 严禁 DTD 实体扩展 |
| `RenderCache` 各分区 | 覆盖率 2048/16 MiB, 段落 256/8 MiB, 字形 2048/16 MiB, 渐变 64/8 MiB, 公式 32/32 MiB, 墨迹 64/32 MiB, 场景 256/8 MiB | `RenderCache::with_limits` 一键应用统一定额；`fork_partition` 可隔离单个分区 |
| GPU 不可变显存驻留 | 64 条目, 64 MiB | 涵盖编码后的主机端比对数据、所有者/条目记录以及设备端有效负载；`GpuContext::set_residency_limits` 接受 `CacheLimits`，条目最多 4096；设置为 0 彻底禁用驻留 |
| `SceneCache::new` | 仅限制条目数 | 字节计量需要 `with_limits` 配合 `cache_with_cost`；超额数据直接返回而不被保留 |
| `FormulaCompiler` | 32 条目, 64 MiB | 单个源码最长 8192 字节 |
| `RasterLimits::authoring()` | 16,777,216 px, 256 MiB 活跃字节, 无工作量上限 | `sandbox()` 模式下追加 2 亿工作量单位上限 |

`Budget` 负责统计单次操作准入的光栅像素、活跃字节（含峰值记录）以及计算工作量；调用方在使用完毕后释放瞬态字节，但已消耗的工作量绝不退还。这些限制约束的是准备阶段，而非 Rust 原生回调或 Typst 内部求值耗时。详见 [`budget.rs`](../../../crates/motion-core/src/budget.rs) 与 [`cache.rs`](../../../crates/motion-core/src/authoring/context/cache.rs)。

每一帧即使在显卡缓存已经完全预热的情况下，依然会保守地为不可变资源预留一次未命中的预算（包括上传暂存和有界哈希查找/碰撞检测工作量）。因此，当前显存缓存的占用情况绝不会改变新帧的准入判定。缓存的驻留配额独立于 `RasterLimits`：如果单帧预算充足，即使某个超大资源无法被驻留缓存保留，它依然能够正常被渲染。正在排队等待绘制的帧会在可变工作缓冲区池之外锁定其常驻资源，因此发生缓存逐出或调小配额绝不会错误回收仍在使用的显存。这些在途锁定的资源由其归属帧的预算支撑，而非计入空闲驻留配额。

`GpuContext::residency_stats` 实时报告累计命中数、未命中数、上传设备字节数、逐出数与绕过数，同时展示当前保留的条目数与逻辑字节数。清空资源会释放缓存所有权，但绝不重置累计计数器，也不会使在途任务失效。显存驻留是绑定在单个 GPU 上下文上的，绝非全局共享缓存，更不是探测或解码外部资源的隐式通道。

## 延迟加载与即时加载

| 延迟模式 Deferred (仅保留路径/选项) | 即时模式 Eager (立即执行耗时工作) |
| --- | --- |
| `Assets::load`, `Image::open`, `Svg::open`, `Svg::new(handle)` | `ImageAsset::from_bytes`, `SvgAsset::from_bytes_with_fonts`, `Assets::preload` |
| `Formula::typst` | `Formula::asset`, `FormulaCompiler::compile` |
| 编译阶段对 `Text` 执行排版塑形 | `Text::layout(&fonts)` 用于提前测量自检 |

`Handle::state()` 在不触发加载的前提下返回当前的 `AssetState`；其状态迁移为：`Unloaded` → `Loading` → `Ready(Arc<A>)` 或 `Failed(Arc<Error>)`。失败状态具有粘性（Sticky），系统绝不隐式重试。多个并发调用方会汇合至同一个解码任务中；在解码器中触发的 panic 会直接固化为粘性资产错误。递归加载解析或嵌套超过 64 层直接报错。未显式指定编译器的 `Formula` 默认使用进程级内嵌字体的 `FormulaCompiler`，因此它不会感知通过 `Assets::with_fonts` 注册的外部字体；若需使用自定义数学字体，请调用 `FormulaCompiler::with_fonts`。

## 线程模型与锁中毒处理

`Asset`、`FontEnvironment`、`Assets`、`Handle` 与 `RenderCache` 均实现 `Send + Sync`（或要求其包装的内容满足），可在多个工作线程间自由传递共享。不同对象的锁中毒处理策略如下：

- `Assets`、`ResourceStore`、`RequestDiagnostics`、`FontUsage` 以及字体使用日志在遇到锁中毒时**自动恢复**并继续执行纯数据操作。
- `SceneCache` 视锁中毒为严重 Bug 并直接触发 panic。
- 字体排版引擎遭遇锁中毒时返回 `ValidationError`（“Font engine is poisoned”）；公式编译器遭遇锁中毒时返回 `FormulaCompileError`；音频声明锁遭遇锁中毒时返回校验错误。
- 重计算操作一律在持有缓存锁的临界区之外执行，因此某个耗时较长的构建闭包绝不会阻塞其他不相干条目的并发访问。

## 故障处理与诊断

资源加载失败会明确携带资产类型与数据源名称。`Handle::get_with` 将错误写入显式的 `RequestDiagnostics`（基于 `Arc` 身份自动去重），该结构支持在线程间安全传递；不存在隐式的单帧全局上下文。常规生产级渲染默认将首个发生的资源诊断作为带帧号标记的错误直接返回，除非在配置中显式开启 `RenderOptions::allow_missing_assets(true)`（Studio 预览模式默认开启此项）；此时 `Image` 会绘制洋红色占位图并记录诊断信息。`RenderOptions::mark_invalid_geometry(true)` 针对几何异常提供了对称策略：校验失败的叶子几何图形会在其局部原点绘制一个洋红色方块，而非直接导致整帧渲染崩溃。Studio 预览模式默认启用该特性；静帧导出、视频导出与 CLI 命令行始终保持严格报错的默认行为。`ExportReport.fonts` 依序列出实际被使用的字体，并附带非密码学源哈希值，其展示顺序独立于后台工作线程的完成次序。

## 在 Studio 与 CLI 中配置资源

Studio 与 CLI 两大入口均消费注册在 `Project` 上的 `RenderOptions`（`Project::render_options`），其中携带了 `resources: Option<Assets>`、`RenderCache` 以及 `RasterLimits`。CLI 命令行本身不提供指定资产根路径或字体文件的通用 flag（参见 [`cli.rs`](../../../crates/motion/src/project/cli.rs) 中的 `--help`），而 `studio` 启动时直接基于这些选项构建其 Composition，因此项目应当直接在 Rust 代码中配置根路径与字体：

```rust,no_run
use motion::prelude::*;

fn main() -> motion::Result {
    let mut fonts = FontLibrary::new();
    fonts.load_file("assets/project.ttf")?;
    let assets = Assets::new("assets")?.with_fonts(fonts);
    let _options = RenderOptions::default().resources(assets);
    Ok(())
}
```

基于这些选项克隆出的各个 Composition 共享同一个 `Assets` 所有者（及其背后的资源存储）；不同渲染会话则独立维护各自的输出目标与局部缓存。

## 相关参考

- [内容系统教程](../tutorial/06-content.md)
- [内容系统契约](content.md)、[系统可靠性](reliability.md)、[渲染与导出设计](rendering.md#资源与内容)
- 源码实现：[`asset.rs`](../../../crates/motion-core/src/asset.rs)、[`context.rs`](../../../crates/motion-core/src/authoring/context.rs)、[`partition.rs`](../../../crates/motion-core/src/authoring/context/partition.rs)
- 字体与公式：[`font.rs`](../../../crates/motion-content/src/font.rs)、[`options.rs`](../../../crates/motion-render/src/options.rs)、[`compiler.rs`](../../../crates/motion-formula/src/compiler.rs)
