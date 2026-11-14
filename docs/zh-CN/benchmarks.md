# 性能基准测试

`motion-bench` 拥有四个基准测试目标。它们回答不同的技术问题，因此它们的测量结果绝不合并为一个单一指标。

| 测试目标 | 回答的问题 | 测试驱动 Harness | 固化基准线 |
| --- | --- | --- | --- |
| `performance` | 针对性优化是否改变了 640x360 或 1080p 下的 29 个参考测试组表现？ | 自定义脚本 | 是，见[当前实测数据](#当前实测数据) |
| `films` | 实际的 Gallery 完整短片渲染需要耗时多久？ | 自定义脚本 | 否 |
| `criterion_frames` | 针对少量 CPU 渲染帧的快速统计检验。 | criterion | 否 |
| `breadth` | 引擎跨内容类型、分辨率、流水线阶段与图元的性能开销特征是什么？ | 自定义脚本 | 否 |

`breadth` 的存在是为了在不干扰已固化的 `performance` 工作负载的前提下扩展测试覆盖：`performance` 会对其自身源码计算指纹哈希，修改其用例会导致已登记的基线测试作废。向 `performance` 添加用例会改变其 `workload_fingerprint`，使得所有历史记录失去可比性；除非刻意启动新的测试序列，否则新用例应添加到 `breadth` 中。

## 电影级特性覆盖

breadth 目标目前包含 `filter-bloom`、`filter-lens`、`filter-lut` 以及 `display-transform`（AgX 色调映射加上带种子的编码胶片颗粒）。这些用例不改变已固化的 performance 测试组。本实现仅扩展测试覆盖，未执行性能对比测试。功能与原生一致性以及 P6 画质测量详见[画质评测报告](quality-report.md)。

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

## 运行 `breadth`

```sh
cargo bench -p motion-bench --bench breadth                      # CPU
cargo bench -p motion-bench --bench breadth --features gpu       # CPU 与 GPU
```

测试结果输出至 `target/performance/breadth-<timestamp>/`（可通过 `MOTION_BENCH_OUTPUT` 覆盖），采用与其他目标相同的 `report.json`、TSV 与参考基准图布局，因此 `MOTION_BENCH_COMPARE` 可直接兼容使用。通用的 `MOTION_BENCH_SAMPLES`、`MOTION_BENCH_WARMUPS` 与 `MOTION_BENCH_ITERATIONS` 环境变量同样生效；使用 `MOTION_BENCH_SAMPLES=5` 可执行快速冒烟测试。

| 环境变量 | 默认值 | 作用说明 |
| --- | --- | --- |
| `MOTION_BENCH_CASES` | 全部 | 逗号分隔的渲染用例列表。 |
| `MOTION_BENCH_BACKENDS` | `cpu`（带特性时为 `cpu,gpu`） | 待测量的后端。 |
| `MOTION_BENCH_SIZE` | `640x360` | 渲染用例的像素尺寸；设计尺寸保持 640x360。 |
| `MOTION_BENCH_SCALING` | `1` | 在 `MOTION_BENCH_SIZES` 指定的每个尺寸下重复运行 `SCALING_CASES`。 |
| `MOTION_BENCH_SIZES` | `1280x720,1920x1080` | 缩放测试组的尺寸列表，单边最大支持 4096。 |
| `MOTION_BENCH_STAGES` | `1` | 测量 `STAGE_CASES` 各 CPU 流水线阶段的耗时。 |
| `MOTION_BENCH_SEQUENCES` | `1` | 记录顺序回放与随机 Seek 定位耗时。 |
| `MOTION_BENCH_MICRO` | `1` | 记录图元微基准测试。 |
| `MOTION_BENCH_FONT` | 仓库内嵌 Noto Sans 固件 | 文本用例所使用的字体文件。 |

### 测量涵盖的内容

- **渲染用例 (24 项)。** 包含渐变（线性、径向、扫描）、12 种混合模式、路径裁剪/嵌套裁剪/形状裁剪、虚线/粗线/细发丝描边、自相交奇偶绕数路径、嵌套与宽幅隔离图层、1k/10k/50k 节点场景、800 个叶子节点的 Row/Column 布局、大半径模糊、色差（Chromatic Aberration）、三级滤镜链、长段落文本排版、200 个标签、2 万弹道粒子与 6 千物理模拟粒子，以及 3 个并列模糊图层。每个用例针对各后端分别记录 `warm-frame`、`sequential-playback` 与 `random-seek` 指标。在开始计时前，会在一次无关 Seek 后重放 3 帧，必须达到逐像素严格一致，且画面不得为纯色死平帧。
- **内存输出。** `filter-siblings` 还会额外记录 `memory-frame`，即使用自适应分带的直通 RGBA8 `render_output_at` 路径，并与 `render_frame` 的结果对比校验（CPU 上要求位完全一致，GPU 上要求在容差范围内）。
- **分辨率缩放。** 在各配置分辨率下测量 `nodes-10k`、`gradient-linear`、`blend-modes` 和 `blur-large`，记录为 `<backend>/<case>@WxH/warm-frame`。
- **阶段耗时。** 针对 9 个 CPU 用例测量细分阶段：`build`、`compile-fresh-node`、`compile`、`prepare`、`linear` 和 `output`。其中 `prepare` 在禁用覆盖率缓存的情况下运行，以便揭示热身帧所掩盖的冷启动开销。
- **微基准测试。** 包括 sRGB 色彩往返、Oklab 色彩插值、变换矩阵组合、SVG 路径解析、描边轮廓提取（未缓存 vs 缓存命中）、确定性随机数生成、噪声生成、Row/Column 布局测量、4 条音轨带 44.1 至 48 kHz 重采样的混音、1080p 下的 RGBA8 转换与 PNG 编码，以及标签排版。
- **预算拒绝。** 触发引擎准入限制（`StructuralLimit`）会被记录在 `rejected.txt` 中而不会中断整个运行，因为“该场景超出预算被拒绝”本身是一个有效的基准结论。

### 功能检查

`cargo test -p motion-bench --test breadth_cases` 在 160x90 分辨率下渲染每个轻量用例，检查确定性与非平坦输出而不进行计时。在未优化 Debug 构建下耗时数秒的繁重用例置于被忽略的测试中：

```sh
cargo test --release -p motion-bench --test breadth_cases -- --include-ignored
```

## 非短片全量基准重测

在基准系统重构后，使用本流程针对当前源码建立测量。保持 `films` 禁用，显式选择 `performance`、`breadth` 和 `criterion_frames`；`--all-features` 会开启短片测试门控，此处不应使用。若必须清理过时的历史数据，在开始前使用独立的[重置基准历史](#重置基准历史)，切勿在这些运行之间清空。

```sh
cargo bench --locked --offline -p motion-bench --features gpu,ffmpeg --bench performance --bench breadth --bench criterion_frames --no-run
MOTION_BENCH_FONT=tests/fixtures/fonts/NotoSans-Regular.ttf cargo bench --locked --offline -p motion-bench --features gpu,ffmpeg --bench performance
cargo bench --locked --offline -p motion-bench --features gpu,ffmpeg --bench breadth
cargo bench --locked --offline -p motion-bench --features gpu,ffmpeg --bench criterion_frames
```

自定义目标默认使用 2 次预热、101 个样本且每样本 1 次迭代；覆盖任何参数均需记录。序列帧测量遵循其配置的逐帧时序，而非热身帧样本数。Criterion 使用其自身的采样/预热/测量参数。保持各目标的测量结果独立，记录预算拒绝情况，仅登记通过健全性验证的报告。这些测量数据不代表 Studio 的屏幕呈现帧率、进程 RSS 内存占用或全片导出验收结果。比对规则详见[结果解读](#结果解读)。

## 可选的聚焦对照矩阵

`benches/baseline.cjs` 固化了 1080p 下的 14 个 CPU/GPU 测试组：空白对照、基础几何、SVG、文本排版、10000 个轨道粒子、长墨迹笔画以及 60 帧 H.264 视频导出。每个健康的测试组需要对其固化的程序执行 3 次独立运行。单帧/操作阶段使用 101 个样本；完整导出阶段使用 5 个样本且无法确立 P95。既有的 1 GiB/200 亿工作量基准配额策略保持不变。测试失败会保留日志并直接上报，绝不盲目调大上限或删除用例。

```sh
cargo bench --locked --offline -p motion-bench --bench performance --features gpu,ffmpeg --no-run
node benches/baseline.cjs --phase probes --state target/performance/baseline.json --program <performance-executable>
node benches/baseline.cjs --phase repeats --state target/performance/baseline.json
node benches/baseline.cjs --phase register --state target/performance/baseline.json
```

这个可选矩阵并不等同于非短片全量基准重测，也不会被其隐式调用。它使用 `performance` 目标而非 `films`，但包含了视频导出；仅在有意测量该导出性能时才运行。在登记之后，请遵循[报告更新流程](#更新流程)。

## 重置基准历史

仅当源码或工作负载发生重大重构导致当前基准线失效时，才在创建全新测量之前执行重置。切勿在测量进行过程中或日常的对照组/候选组对比时执行重置；这会删除保留的原始实测证据。

```sh
node benches/baseline.cjs --phase reset
```

`reset` 会删除两个被忽略的结果根目录（`target/performance` 与 `crates/motion/target/performance`）并重置 [performance-runs.json](../performance-runs.json)。它绝不删除构建缓存、源码、字体或常规测试日志。已提交的 Git 历史依然可以找回；已被删除的未跟踪原始产物无法找回。

## 当前实测数据

基准系统重构后，历史的冒烟测试耗时已被全部移除；它们不能作为当前源码或工作负载的基准线。下方生成的报告记录了经过验证的当前运行数据、采样参数、运行环境与任何被拒绝的用例。本轮刷新仅记录了 `performance`；`breadth` 与 `criterion_frames` 被顺延，`films` 被刻意排除。切勿从这些单帧、流水线阶段与微基准测量数据中推断短片导出耗时或 Studio 运行帧率。

登记的基线数据早于 R3-8 滚动式 CPU 模糊算法修改。它依然作为其固化可执行文件的历史证据。下方单独的聚焦对比使用全新的对照组/候选组运行；它绝不会将最初的全量测试耗时强行当作该优化的实测值。

### R3-8 配对检验

在 Apple M3/Metal 硬件平台、640x360 Final 质量配置下执行了 6 组交替的 AB/BA 配对测试，每组 2 次预热与 101 个采样。仅针对 Empty、Shapes、Blur、Shadow、Bloom 进行了测量（开启 CPU 各阶段）；禁用序列帧、微基准、冷启动会话与视频导出。两个固化的可执行文件在所有测试运行中均生成了位完全一致的参考像素和校验和。

| CPU 热身单帧 | 对照组 P50 (ms) | 候选组 P50 (ms) | 配对提升幅度 (95% 置信区间) | 评估结论 |
| --- | ---: | ---: | --- | --- |
| Blur（小卷积核） | 13.228 | 27.372 | -106.70% [-116.20, -100.56] | 性能衰退 (regressed) |
| Shadow（sigma 2） | 17.301 | 28.136 | -64.78% [-65.86, -58.27] | 性能衰退 (regressed) |
| Bloom（sigma 8） | 83.408 | 46.990 | +42.88% [41.05, 45.79] | 性能提升 (improved) |

展示的 P50 数值为跨运行的中位数；提升幅度为配对相对变化率的中位数，而非两个展示中位数的简单比例。CPU 阶段线性测量验证了相同的变动趋势。Empty/Shapes 以及 GPU 热身单帧的 P50 变动不显著（uncertain）；GPU Empty 的 P95 出现轻微临界衰退（-1.12%，置信区间 [-1.56, -0.01]），保留在报告中。

这第一版全滚动式实现未通过性能验收：较小的均匀卷积核发生了显著衰退，需要进一步优化。切勿对这些结果进行平均化计算，或将其推断至完整短片、Studio FPS、更大 sigma/更高分辨率或进程 RSS 内存。所有 50 个用例/阶段的结果均记录在本地未跟踪的 `target/performance/r3-8-1793849842040/` 报告中；未完成或磁盘空间不足的尝试不计入配对。

### R3-8 小卷积核跟进优化

均匀半径 1 和 2 目前采用直接的整数加权采样点；更大半径保持补偿式滚动路径。6 组新的交替 AB/BA 配对测试采用相同的配置，并以最初未应用 R3-8 的代码作为对照组，而非全滚动式候选组。该测试组单独登记，不与上方第一轮对比数据混杂。

| CPU 热身单帧 | 对照组 P50 (ms) | 候选组 P50 (ms) | 配对提升幅度 (95% 置信区间) | 评估结论 |
| --- | ---: | ---: | --- | --- |
| Blur（小卷积核） | 13.138 | 5.520 | +57.97% [57.26, 58.41] | 性能提升 (improved) |
| Shadow（sigma 2） | 16.949 | 7.477 | +55.89% [55.59, 56.24] | 性能提升 (improved) |
| Bloom（sigma 8） | 81.626 | 46.645 | +42.91% [42.76, 43.35] | 性能提升 (improved) |

全部 3 个用例的 CPU P95 以及阶段线性估算均同步提升。CPU Empty/Shapes 热身帧以及全部 GPU 热身帧的 P50/P95 变动不显著。非目标的 Shapes 活跃节点编译 P50 出现临界轻微衰退（-1.39%，区间 [-4.58, -0.01]）；予以保留且未归咎于滤镜逻辑。这有力支持了测量的 CPU 滤镜目标优化，但并不代表无回归的整体系统验收，也不代表短片/Studio 帧率、内存占用或跨设备性能承诺。

全部 12 次运行在参考像素、校验和、配置与固化二进制上严格一致。默认/GPU CPU 滤镜回归测试、生产环境检查以及 53 项原生 GPU 一致性检查全部通过。测量后的源码变动仅限于注释和现有测试；实测的运行时等价性已获验证。所有 50 项结论、两组分位数与溯源信息均已记录在本地未跟踪的 `target/performance/r3-8-small-kernel-1793853335012/` 报告及相关比对文件中。先前的回归记录与失败尝试完整保留。

以下标记界定了由 `node benches/report.cjs` 自动维护的章节；上方的使用指南为手动编写维护。

<!-- performance-report:start -->

以下为当前源码下的非短片基准测试客观观测数据，而非对比已被删除的历史测量数据的优化指标。
已登记的测量数据、验证注释与原始运行路径记录于 [performance-runs.json](../performance-runs.json)。原始运行目录保留了完整的 JSON/TSV、采样点与参考基准图。

### 结果解读

- 时间提升比例 = `(参考基准 - 当前值) / 参考基准 * 100%`。正值代表速度变快；负值代表性能衰退。
- “Previous”与同一序列中紧邻的上一步骤对比。“Initial”始终与 step 00 初始基准对比。
- 表中数值为独立运行之间的中位数；MAD% 反映不同运行之间的离散度。Batch P95 绝不可称为单帧长尾延迟。
- 对比上一阶段的提升评估采用全新的交替 A/B 后 B/A 顺序的配对对照组。对比历史基线仅作为描述性参考，不代表因果推断。
- 至少累计 5 组配对测试后，配对 P50 变化量才会计算确定性 Bootstrap 置信区间。样本较少时属于探索性评估；跨越 0 的置信区间判定为不确定（uncertain）。
- P95 计算要求每次运行中包含至少 100 个单帧/单次操作的独立样本；否则显式标识为样本量不足。
- 在同一测试序列内，用例集合、分辨率、画质等级、预算配额、字体、特性组合、采样参数、宿主机/编译器版本以及图形适配器必须保持完全一致。
- 校验和、精确的参考基准像素与原始样本汇总均经校验。逻辑峰值字节/工作量不代表物理进程 RSS 内存或墙钟时间。
- 不提供跨用例的平均加速比。冷启动会话、热身单帧、Seek 跳转、I/O 读取与编码阶段耗时始终保持相互独立。

### 当前测量记录

源码 Git Commit：9cb8a9b1e5e3c4f72e817cbd259a5aaf47ef070c。

测试目标：performance。排除项：breadth、criterion_frames、films。

全套初始基线执行一次调用，CPU 与 GPU 在同一个进程中运行：2 次预热、101 个热身单帧样本且每样本 1 次迭代；回放与 Seek 跳转各使用 90 帧。热身单帧具备 P95 资格，序列帧与 5 项批均值微基准测试不具备。此外，R3-8 在 640x360 分辨率、双后端下对 Empty/Shapes/Blur/Shadow/Bloom 执行了 6 组全新的交替 AB/BA 配对测试，开启 CPU 阶段测量，禁用序列帧/微基准/视频导出/冷启动。针对小卷积核另外执行了独立的 6 组配对测试，将新候选实现与 R3-8 引入前的同一初始对照组对比；两组测试数据不合并计算。这些针对性配对提供了单用例维度的不确定性评估，而非全测试套件的加速比宣称。

宿主机环境：Apple M3 (Metal GPU), macOS, aarch64, release, 640x360 Final 画质, 内嵌 Noto Sans 字体, 72 个粒子, 1 GiB/200 亿工作量基准策略。测量了全部 29 个用例、双后端、回放/Seek 以及 5 项微基准测试；禁用了冷启动会话、阶段耗时与视频导出。在固化的构建中启用了 GPU 与 FFmpeg，未启用 films。运行时与工作负载源码与记录的 Git 版本完全匹配；构建后的修改仅包含对 `benches/report.cjs` 与 `benches/baseline.cjs` 的纯报告/归档调整，以及 `crates/motion-bench/Cargo.toml` 中仅用于调度的 films 特性门控。Breadth 产物不全且与 Criterion 共同顺延；二者均未登记。本数据不代表 Studio 屏幕帧率、进程 RSS 内存占用、全片导出验收通过、跨设备表现，也不代表对已删除历史数据的改进。验证详情参见：`target/performance/non-films-1793845901725/source-verification.json`。

已清理的历史测试序列：14 组。

### 结论摘要

每个测试序列展示一行，使用其最新步骤的数据。结论统计的是用例/阶段对比数量，而非跨用例均值。代表性变动展示至多两项最大的绝对 P50 变化量；其余所有结果与负向对照在详细报告中均可查阅。

| 测试序列 / 最新步骤 | 运行次数 | 用例数 | P50 结论分布 | P95 合规用例 | 代表性耗时 / P50 变动 |
| --- | ---: | ---: | --- | ---: | --- |
| [Engine / 640x360 / Apple M3](../performance-runs.json) / 00 current baseline | 1 | 179 | baseline (无加速比宣称): 179 | 58/179 | Cpu/empty/warm-frame: P50 1.255 ms; P95 1.914 ms<br>Cpu/shapes/warm-frame: P50 2.729 ms; P95 3.448 ms |
| [R3-8 focused / 640x360 / Apple M3](../performance-runs.json) / 01 R3-8 rolling blur | 6 | 50 | uncertain: 43; regressed: 4; improved: 3 | 50/50 | Software/blur/stage-linear: -122.76% [-132.61%, -114.39%]; -35.59%<br>Cpu/blur/warm-frame: -106.70% [-116.20%, -100.56%]; -43.82% |
| [R3-8 small kernels / 640x360 / Apple M3](../performance-runs.json) / 01 R3-8 small-kernel direct taps | 6 | 50 | uncertain: 36; improved: 13; regressed: 1 | 50/50 | Software/blur/stage-linear: +66.26% [+66.18%, +66.44%]; +79.45%<br>Software/shadow/stage-linear: +59.40% [+59.27%, +59.68%]; +66.64% |

### 原始数据与详细报表

活跃的清单文件完备保存了已登记的运行、变动说明、验证信息与回归注释。运行产物目录包含校验完备的样本数据、运行配置、可执行文件环境溯源与参考基准图像；它们是本地临时产物，而非随库分发的性能承诺。已被移除的历史跟踪数据可从 Git 历史中找回，不保留在活跃测量数据行中。

可通过以下命令生成完整的单用例明细表格、MAD 偏差、增量与初始差异、P95 合规性以及步骤注释，无需扩展本页面：

```sh
node benches/report.cjs --details --output target/performance/details.md
```

### 更新流程

1. 保留初始运行目录与固化的可执行程序文件。在每次针对性优化前后，在完全相同的测试组上执行测量。
2. 在 [performance-runs.json](../performance-runs.json) 中登记原始目录路径、变动详情、验证结果与负向对照说明。针对不同工作负载或宿主机硬件开启新的测试序列。
3. 执行 `node benches/report.cjs`。此命令会验证每个已登记的运行并重新生成精简的结论摘要表格；配合 `--details` 与 `--output` 可输出完整报表。
4. 查阅[开发规划](plan.md#性能优化)了解后续的优化任务排期与各项质量门禁。

<!-- performance-report:end -->
