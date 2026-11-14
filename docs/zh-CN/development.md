# 开发与维护检查

本文档是代码编译、测试套件、固件与持续集成（CI）的维护指南。
以下命令均为可选的本地验证流程；查阅文档本身不执行任何命令。关于失败与所有权保证请参见[可靠性契约](design/reliability.md)，关于工作负载与测量规则请参见[性能基准](benchmarks.md)。

## 持续集成 (CI)

工作流定义位于 `.github/workflows/` 中，并在 Pull Request 以及向 `main` 分支推送时运行。PR 上的新提交会自动取消该工作流的上一次运行；所有工作流均拥有只读的 `contents` 权限。

| 工作流 | 作业任务 | 运行环境 |
| --- | --- | --- |
| Structure | 工具自测、`check-structure`、`check-expects`、`check-docs`、`cargo fmt`、`rust-format --check` | Linux |
| Rust | 开启全部特性的 Clippy（`-D warnings`）、工作区测试、Rustdoc 文档测试（包括编译教程篇章）以及严格的 `cargo doc`；无默认特性的教程 doctest；`motion-content` 四组特性单元测试 | 全特性使用 macOS，其余使用 Linux |
| Facade Features | `tools/check-features.cjs` 驱动的 24 个独立 facade 消费者构建 | Linux，以及需要 FFmpeg 的 macOS |

原生 FFmpeg 作业通过 Homebrew 安装 FFmpeg、`pkg-config` 与 LLVM。CI 仅编译并运行未被标记为 `#[ignore]` 的测试；标记为 ignored 的 GPU、字体与 FFmpeg 固件仍需在本地显式执行，CI 绿灯并不证明 Windows 或其他显卡适配器上的运行时支持。`.github/dependabot.yml` 每周提出 GitHub Actions 和 Cargo 依赖更新，PR 使用 `.github/pull_request_template.md` 模板。

## 自动化检查命令

源码规范详见[代码规范](style.md)；`node tools/rust-format.cjs` 应用 rustfmt 及元素间距规则，结构检查脚本强制执行文件布局。

```sh
CARGO_INCREMENTAL=0 cargo check --locked --offline -j 2 --workspace --all-features --all-targets
CARGO_INCREMENTAL=0 cargo clippy --locked --offline -j 2 --workspace --all-features --all-targets -- -D warnings
CARGO_INCREMENTAL=0 cargo test --locked --offline -j 2 --workspace --all-features --lib --bins --tests --examples
CARGO_INCREMENTAL=0 cargo test --locked --offline -p motion-core -p motion-render --test properties
CARGO_INCREMENTAL=0 cargo test --locked --offline -j 2 --workspace --all-features --doc
node tools/check-structure.cjs && node tools/check-expects.cjs && node tools/check-docs.cjs && node tools/rust-format.cjs --check
RUSTDOCFLAGS="-D rustdoc::broken_intra_doc_links -D rustdoc::private_intra_doc_links" cargo doc --locked --offline --workspace --all-features --no-deps
```

`docs/en/tutorial/*.md` 中的 Rust 代码块作为 facade crate（`crates/motion/src/lib.rs`，`tutorial` 模块）的 Rustdoc 测试进行编译，每章根据其所需的 Cargo 特性进行门控。在涉及文件读写、GPU 或窗口的代码中使用 `rust,no_run`，非 Rust 语法使用 `text` 或 `sh`；未标记的栅格会被当作 Rust 测试进行编译。

此外还需检查最小 facade 以及不带场景 GPU 的 Studio，包括 `cargo check --locked --offline -p motion-encode --no-default-features`。facade feature CI 编译的是独立的消费者而非工作区测试目标，因此无关包和 dev-dependencies 无法掩盖缺失的特性。在本地使用 `node tools/check-features.cjs --offline` 运行同样的 24 组矩阵，或使用 `--features gpu,particles --offline` 运行指定用例。每个临时消费者禁用默认特性并仅依赖 facade；`default` 显式测试默认特性集。工作区锁文件会在可用时复用，构建产物在 `target/feature-consumer` 中复用。CI 在 Linux 上检查非原生用例，在 macOS 上检查依赖 Homebrew FFmpeg 开发库的用例。这些均为编译检查，而非 GPU/编解码器的运行时测试。独立的 CI 作业在 macOS 上运行工作区单元/集成/示例测试与 Rustdoc 测试，并在 Linux 上运行不含文本的 content 测试。被忽略的原生测试仍需手动显式指定。使用 `node tools/check-features.cjs --self-test` 测试该驱动脚本。

光栅化内核在 `RasterScope` 和 `Band` 中携带预算、错误路径和分带几何信息，因此没有残留的 `too_many_arguments` 抑制规则；生成的 FFmpeg ABI 声明仅从类型复杂度 lint 中排除。第三方未来的不兼容性报告不算作项目源码的 lint 失败。

### 生产环境 Panic 清单

运行 `node tools/check-expects.cjs` 查看计数，或添加 `--json` 查看每个保留的生产环境 `expect` 及其文件、行号、分类和责任原因。驱动脚本在扫描前会自测词法分析与 cfg 处理；未登记的出现会导致检查失败。脚本会排除测试代码、遮罩字符串/注释，并保守地包含所有特性与目标分支的并集。这是源码级清单，而非 AST 分析或宏展开，不统计 `unwrap` 或 `panic!`。修改原因时请仔细审查周围的控制代码：匹配到消息并不证明不变式成立。注意区分已校验的私有状态、文档已记录的便利性 panic、中毒快速失败策略以及内嵌源码假定；后三者并不代表故障绝不可能发生。

核心和光栅化属性测试使用 `proptest`，每项属性 128 个用例，支持缩小（shrinking）和失败持久化。覆盖有符号有理数帧往返、有限/无限 `zip` 与有限 `then` 的时长/边界规则、像素级独立矩形面积与紧凑 span/strip 的等价性，以及与 CPU 输出一致的有限边界多边形覆盖率。`PROPTEST_CASES` 和 `PROPTEST_RNG_SEED` 可用于提高覆盖率或重现运行；生成的回归用例应妥善保留。

显式固件测试在指定处需要受支持的图形适配器和原生 FFmpeg 编解码器。段落回退和 Unicode 断行测试使用内嵌的 [Noto 固件](../../tests/fixtures/fonts/README.md)，在无需系统字体或环境变量的情况下运行。

对于被忽略的字体测试固件，将 `MOTION_TEST_FONT` 设置为内嵌的 `tests/fixtures/fonts/NotoSans-Regular.ttf`，将 `MOTION_TEST_FALLBACK_FONT` 设置为 `tests/fixtures/fonts/NotoSansCJKsc-Regular.otf`。纯中文固件需要将 CJK 字体作为 `MOTION_TEST_FONT`。彩色位图 Emoji 固件还需要固件 README 中描述的 `MOTION_TEST_EMOJI_FONT`。通过全名选择被忽略的测试。请勿将忽略的测试、无测试匹配的过滤器或成功的编译等同于运行时验收通过。

像素对比检查要求非空白的主体区域、指定命名的[视觉验收](design/reliability.md#视觉验收标准)级别或档位、精确的 CPU 定位重放以及 Replay 档位的 GPU 重放。音频固件检查实际解码的采样与时间戳；视频固件解码帧数与元数据。任何测试均不应调用 ffmpeg/ffprobe 外部进程可执行文件。原生窗口交互、真实输出设备及其他平台需要单独验证。

### 独立后端验收

原生 GPU 固件独立准备 CPU 与 GPU 覆盖率，并根据[视觉验收](design/reliability.md#视觉验收标准)档位进行比对；几何用例必须报告独立的 GPU 覆盖率。

```sh
cargo test --locked --offline -p motion-render --all-features --lib gpu:: -- --include-ignored --test-threads=1
```

渲染器测试套件包括两种绕数规则下的 24 个固定随机种子、精确的重复奇偶抵消、tile 拼缝、大坐标、保护带滤镜、程序化画刷、解析轨迹、内存驻留以及精确的字节/工作量/设备限制准入。现有的 CPU 参考测试和黄金标准样本保持不变。GPU 固件仅认证实际使用的适配器。涵盖文本、SVG、裁剪和粒子的超大场景验收属于规划事项。

## 帧时间诊断

宽度与性能基准测试接受 `MOTION_BENCH_TIMINGS=1` 参数，用于收集逐帧的主机阶段观测值和可用的 GPU 时间戳。这与测量选定 CPU 流水线阶段的 `MOTION_BENCH_STAGES` 区分开来。时间诊断需要 `MOTION_BENCH_ITERATIONS=1`，从记录的阶段数组中排除预热阶段，且默认处于禁用状态。请使用独立的 `MOTION_BENCH_OUTPUT` 目录；切勿将打桩记录的墙钟耗时当作优化验收依据。禁用此开关时，常规报告保留其原始指标。

主机编码包含命令编码与提交；主机回读包含完成等待、映射与提取。设备时间区间与这些主机观测有重叠，切勿将它们累加。保留所有原始设备数据，但如果某个时间区间长于其外层帧的墙钟时间，应标记为不可信。数据看似合理并不能证明时间戳正确。诊断 JSON 会保留原始 ticks，并区分无效区间、未执行的 pass 以及不支持时间戳的设备。打桩仪表的负载数据与逻辑光栅预算分开报告，并不代表物理内存用量。在构建庞大的可选特性之前请检查可用空间，切勿在未明确决定的情况下删除固化的基准测试产物来腾出空间。
