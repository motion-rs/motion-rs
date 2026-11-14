# 性能基准测试

`motion-bench` 拥有三个基准测试目标。它们回答不同的技术问题，因此它们的测量结果绝不合并为一个单一指标。

| 测试目标 | 回答的问题 | 测试驱动 Harness |
| --- | --- | --- |
| `performance` | 引擎在 60 个核心与广度用例、多种分辨率、流水线阶段与图元上的开销是多少？一次改动是否改变了它们？ | 自定义脚本 |
| `films` | 实际的 Gallery 完整短片渲染需要耗时多久？ | 自定义脚本 |
| `criterion_frames` | 针对少量 CPU 渲染帧的快速统计检验。 | criterion |

仓库中不固化也不登记任何基准线。原始运行目录是被 Git 忽略的本地产物；改进或回退的结论来自同一台机器上的配对对比（见[对比两个构建](#对比两个构建)）。

## 电影级特性覆盖

广度用例包含 `filter-bloom`、`filter-lens`、`filter-lut` 以及 `display-transform`（AgX 色调映射加上带种子的编码胶片颗粒）。功能与原生一致性以及 P6 画质测量详见[画质评测报告](quality-report.md)。

## 可选的短片基准测试

`films` 目标需要开启 `films` 特性，该特性默认不开启。在常规的基准测试命令（包括 `--benches`）中 Cargo 会默认跳过它。在需要时显式运行：

```sh
cargo bench -p motion-bench --bench films --features films            # CPU
cargo bench -p motion-bench --bench films --features films,gpu        # CPU 与 GPU
cargo bench -p motion-bench --bench films --features films,gpu,ffmpeg # 包含原生视频导出
```

Gallery 目录中的每个短片对应一个 `films` 用例；可通过 `MOTION_BENCH_CASES` 挑选子集。

`--all-features` 同样会启用 `films`；若要排除长耗时的短片测试请避免使用该选项。该开关仅用于目标过滤；GPU 与原生编码依然是独立的特性开关，短片测量的参数保持不变。

## CPU Criterion 帧测试

Criterion 提供了独立的热身 CPU 全帧基准测试，但不能替代固化的可执行文件/参考基准图以及独立的 AB/BA 进程协议：

```sh
cargo bench --locked --offline -p motion-bench --no-default-features --features text,particles --bench criterion_frames
cargo bench --locked --offline -p motion-bench --no-default-features --features text,particles --bench criterion_frames -- --test
```

涵盖 7 个用例：在 640x360 分辨率下的 10,000 个几何图形、两批各 50,000 个（共 100,000 个）粒子、1,824 个文本字符、32 个渐变、200 个隔离图层与 sigma-12 模糊，以及在 3840x2160 分辨率下的 64 个 tile 渲染。
全部使用内嵌的 Noto Sans 字体、显式的 CPU 会话、100016 结构节点上限 `max_nodes`、默认 256 MiB 逻辑字节配额与默认像素配额。粒子和长文本声明了 `max_work=4000000000`；其余用例使用创作默认值而不设工作量上限。基准重构前捕获的报告不能作为这些用例或当前默认配置的参考数据。默认预算超额拒绝不同于实际渲染帧耗时。测量前会先执行环境就绪、首帧及精确重放验证。Criterion 的每个采样包含完整的热身会话帧；它不是冷启动、GPU、P95 或进程 RSS 的衡量依据。Criterion 当前每用例使用 10 个样本、1 秒预热和 3 秒测量时间；其 batch 估算的是单次迭代均摊成本，而非单帧的长尾时延。`--test` 仅作为功能性的冒烟检查。

## 运行 `performance`

```sh
cargo bench -p motion-bench --bench performance                  # CPU
cargo bench -p motion-bench --bench performance --features gpu   # CPU 与 GPU
```

测试结果输出至 `target/performance/performance-<timestamp>/`（可通过 `MOTION_BENCH_OUTPUT` 覆盖），包含 `report.json`、TSV 与参考基准图。该目标需要 `image`、`text`、`svg`、`particles`、`audio` 与 `diagram` 特性，均为默认开启。自定义目标默认使用 2 次预热、101 个样本且每样本 1 次迭代；覆盖任何参数均需记录。使用 `MOTION_BENCH_SAMPLES=5` 可执行快速冒烟测试。

| 环境变量 | 默认值 | 作用说明 |
| --- | --- | --- |
| `MOTION_BENCH_CASES` | 全部 60 项 | 逗号分隔的用例列表，涵盖核心与广度两组。 |
| `MOTION_BENCH_BACKENDS` | `cpu`（带特性时为 `cpu,gpu`） | 待测量的后端。 |
| `MOTION_BENCH_SIZE` | `640x360` | 渲染用例的像素尺寸；设计尺寸保持 640x360。 |
| `MOTION_BENCH_SCALING` | `0` | 在 `MOTION_BENCH_SIZES` 指定的每个尺寸下重复运行 `SCALING_CASES`。 |
| `MOTION_BENCH_SIZES` | `1280x720,1920x1080` | 缩放测试组的尺寸列表，单边最大支持 4096。 |
| `MOTION_BENCH_STAGES` | `0` | 测量所有选中用例各 CPU 流水线阶段的耗时。 |
| `MOTION_BENCH_SEQUENCES` | `1` | 记录顺序回放与随机 Seek 定位耗时。 |
| `MOTION_BENCH_COLD` | `0` | 记录冷启动会话帧。 |
| `MOTION_BENCH_MICRO` | `1` | 记录微基准测试。 |
| `MOTION_BENCH_VIDEO` | `0` | 记录视频导出运行（需要 `ffmpeg` 特性）；`MOTION_BENCH_VIDEO_FRAMES` 设置帧数。 |
| `MOTION_BENCH_PARTICLES` | `72` | 核心粒子用例的粒子数量。 |
| `MOTION_BENCH_FONT` | 仓库内嵌 Noto Sans 固件 | 文本用例所使用的字体文件。 |

### 测量涵盖的内容

- **核心用例 (29 项)。** 几何图形、填充、描边、墨迹、隔离图层、模糊、阴影、图像、遮罩、转场、泛光、调色、运动模糊、粒子、文本、SVG、路径采样与形变，均在 640x360 下渲染。
- **广度用例 (31 项)。** 渐变（线性、径向、扫描）、12 种混合模式、路径裁剪/嵌套裁剪/形状裁剪、虚线/粗线/细发丝描边、自相交奇偶绕数路径、嵌套与宽幅隔离图层、1k/10k/50k 节点场景、800 个叶子节点的 Row/Column 布局、大半径模糊、色差（Chromatic Aberration）、三级滤镜链、泛光/镜头/LUT 滤镜、显示变换、长段落文本排版、200 个标签、弹道/物理模拟/子发射器/碰撞/并行粒子系统，以及 3 个并列模糊图层。
- **每个用例。** 针对各后端分别记录 `warm-frame`、`sequential-playback` 与 `random-seek` 指标。在开始计时前，会在一次无关 Seek 后重放 4 帧，必须达到匹配（CPU 上逐像素一致，GPU 上在视觉容差内），且画面不得为纯色死平帧。
- **内存输出。** `filter-siblings` 与 `display-transform` 还会额外记录 `memory-frame`，即使用自适应分带的直通 RGBA8 `render_output_at` 路径，并与 `render_frame` 的结果对比校验。
- **分辨率缩放。** 在各配置分辨率下测量 `nodes-10k`、`gradient-linear`、`blend-modes` 和 `blur-large`，记录为 `<backend>/<case>@WxH/warm-frame`。
- **阶段耗时。** 针对 CPU 用例测量细分阶段：`build`、`compile-fresh-node`、`compile`、`prepare`、`linear` 和 `output`。其中 `prepare` 在禁用覆盖率缓存的情况下运行，以便揭示热身帧所掩盖的冷启动开销。
- **微基准测试。** 包括 sRGB 色彩往返、Oklab 色彩插值、变换矩阵组合、SVG 路径解析、描边轮廓提取、确定性随机数生成、噪声生成、Row/Column 布局测量、带重采样的 4 条音轨混音、1080p 下的 RGBA8 转换与 PNG 编码，以及标签排版。
- **预算拒绝。** 触发引擎准入限制（`StructuralLimit`）会被记录在 `rejected.txt` 中而不会中断整个运行，因为“该场景超出预算被拒绝”本身是一个有效的基准结论。

这些测量数据不代表 Studio 的屏幕呈现帧率、进程 RSS 内存占用或全片导出验收结果。

### 功能检查

`cargo test -p motion-bench --test performance_cases` 在 160x90 分辨率下渲染每个轻量广度用例，检查确定性与非平坦输出而不进行计时。在未优化 Debug 构建下耗时数秒的繁重用例置于被忽略的测试中：

```sh
cargo test --release -p motion-bench --test performance_cases -- --include-ignored
```

## 一键对比

`benches/run.cjs` 会分别构建并运行对照组与当前工作区的 `performance` 目标，执行交替配对，并输出报告与对照图：

```sh
node benches/run.cjs --control-ref HEAD --pairs 6 --features gpu
MOTION_BENCH_CASES=shapes,blur,text node benches/run.cjs --control-ref main --label blur-opt
```

对照组默认从 `--control-ref`（缺省为 `HEAD`）在临时 git worktree 中构建，也可用 `--control <run-dir>` 指定已有运行目录。所有 `MOTION_BENCH_*` 变量都会透传。输出位于 `target/performance/compare-<timestamp>/`（可用 `--output` 覆盖）：`compare.md` 为对比表，`compare.svg` 为各用例配对 P50 变化量及其 Bootstrap 区间的条形图（`--phase` 选择阶段，默认 `warm-frame`；绿色为提升，红色为回退，灰色为不确定）。`node benches/chart.cjs --pairs <dir> --output <file>` 可基于已有配对目录重绘图表。完整基准会运行 `2 x pairs + 2` 次，建议用 `MOTION_BENCH_CASES` 与 `MOTION_BENCH_SAMPLES` 缩小范围。

## 对比两个构建

保留对照组的可执行文件，并在同一台机器上与候选构建对比。`--control` 与 `--candidate` 各指向一个已完成的运行目录，其中包含 `report.json` 与固化的 `benchmark-executable`。

```sh
node benches/pair.cjs --control <control-run> --candidate <candidate-run> --output target/performance/pairs --label <name> --pairs 6
node benches/report.cjs --pairs target/performance/pairs --output target/performance/compare.md
```

`pair.cjs` 按交替 A/B 后 B/A 的顺序，使用对照报告的配置运行两个可执行文件，并拒绝配置、环境或可执行文件溯源不一致的运行。`report.cjs` 针对每个用例与阶段输出配对 P50 变化量及其确定性 Bootstrap 区间、结论评估、P95 变化量与 MAD。

- 时间提升比例 = `(对照 - 候选) / 对照 * 100%`。正值代表速度变快。
- 区间至少需要 5 组配对；样本较少时属于探索性评估，跨越 0 的区间判定为不确定（uncertain）。
- P95 计算要求每次运行中包含至少 100 个单帧/单次操作的独立样本。
- 用例集合、分辨率、画质等级、预算配额、字体、特性组合、采样参数、宿主机、编译器版本以及图形适配器必须保持完全一致。
- 校验和与精确的参考基准像素均经校验。逻辑峰值字节/工作量不代表物理进程 RSS 内存。
- 不提供跨用例的平均加速比。冷启动会话、热身单帧、Seek 跳转、I/O 读取与编码阶段耗时始终保持相互独立。

## 可选的聚焦对照矩阵

`benches/baseline.cjs` 固化了 1080p 下的 14 个 CPU/GPU 测试组：空白对照、基础几何、SVG、文本排版、10000 个轨道粒子、长墨迹笔画以及 60 帧 H.264 视频导出。每个健康的测试组需要对其固化的程序执行 3 次独立运行。单帧/操作阶段使用 101 个样本；完整导出阶段使用 5 个样本且无法确立 P95。测试失败会保留日志并直接上报，绝不盲目调大上限或删除用例。

```sh
cargo bench --locked --offline -p motion-bench --bench performance --features gpu,ffmpeg --no-run
node benches/baseline.cjs --phase probes --state target/performance/baseline.json --program <performance-executable>
node benches/baseline.cjs --phase repeats --state target/performance/baseline.json
```

该矩阵包含视频导出；仅在有意测量该导出性能时才运行。
