# CPU/GPU 性能对比与 GPU 后端优化方案

日期：2026-11-12。

本文记录已完成的测量与优化方案。O0 诊断正在实施，最新结果见第 10 节；
**本文不表示 O0 已全部完成，也不表示两分支已通过等画质验收。**
第 2–4 节保留此前固定版本的测量，不能与第 10 节的候选构建混为一谈。

## 1. 结论摘要

1. GPU 回退确实存在，但不是所有场景都变慢。28 个有效场景中，17 个 P50
   耗时变化区间在零以上，7 个在零以下，4 个跨零；这些是未做多重比较校正的逐场景统计。
2. 回退较大的场景包括 `path-trim`（+53.4%）、`morph-curves`（+48.9%）、
   `stroke`（+44.3%）和 `path-sampling`（+43.5%）。`image` 的耗时反而减少约 30.3%。
3. 不能确认“固定增加 1.2–1.9 ms GPU 调度开销”的说法。新 coverage 管线的实测设备时间
   随负载变化：`path-trim` 约 0.370 ms，`morph-curves` 约 0.306 ms，
   `stroke` 约 1.624 ms。这些时间包含算法执行，并不是单独测出的 dispatch 开销。
4. `prefix_tiles` 和 `carry_rows` 是明确热点。在 `path-trim`、`morph-curves`、
   `path-sampling` 中，两者约占新 coverage 管线耗时的 70.2%、78.6%、67.3%。
5. warm-frame 下，master 复用已栅格化的 CPU coverage，dev/gpu 的相关路径则重新生成
   设备 coverage。这个结构差异值得优化，但没有完成缓存消融实验，不能给出其因果贡献比例。
6. 主机 encode 和等待 GPU 的成本也有变化。GPU 工作 Buffer 已经池化，
   “每帧重新创建所有 Buffer”不是准确归因。
7. 早期 CPU 后端数据只有单轮，仍只作历史附录；新增的六对 CPU/GPU 测量及其
   候选脏树身份、严格可比性限制和 O0 实施状态见第 10 节。

**建议顺序：先补齐视觉和计时可信性，再分别验证整型 prefix 并行化、row carry
并行化、GPU coverage 结果缓存，随后优化像素积分和主机上传。不要先大改公共 prepared 数据结构，
也不要通过放宽断言、提高预算或隐式切换 CPU 后端获取更好看的数字。**

## 2. 测量身份与协议

### 2.1 本轮 GPU 复测

| 项目 | 实际设置 |
| --- | --- |
| master | `6c89650afdba745c29ffac811610dbe901d42e9b` |
| dev/gpu | `4c38e7d5531b87693ae13ff8e0d6e3a5d2322f2f` |
| CPU / GPU | Apple M1 Max，10 个逻辑 CPU；Metal GPU 后端 |
| 内存 / 系统 | 32 GiB；macOS，Darwin 25.6.0 arm64，build 25G83 |
| 工具链 | rustc 1.99.1，LLVM 23.1.1，`aarch64-apple-darwin` |
| 构建 | release，opt-level 3，`motion-bench --features gpu`，locked/offline |
| 场景 | 相同 artwork 源码；640×360，Final，particles = 72 |
| 入口 | `performance` benchmark 的 `Gpu/<case>/warm-frame`，返回 CPU 可读输出 |
| 正式测量 | 6 对独立进程，按 AB/BA 交替，每个场景独立启动；轮换场景顺序 |
| 每进程 | 2 次 warmup，101 samples，iterations = 1 |
| 诊断测量 | 另外 3 对独立进程，每进程 2 次 warmup + 21 samples |
| 插桩 | 正式测量关闭；诊断测量开启，分析时去掉诊断数组前 2 个 warmup |
| 关闭项 | cold、sequence、独立 CPU stage benchmark、micro、video |

正式测量和诊断测量在 2026-11-12 14:36–14:55（UTC+8）执行。两个冻结二进制
分别来自固定提交的隔离工作树；诊断补丁不改变场景、断言或预算。工作树已经清理，
补丁、二进制、原始报告仍保留。

冻结二进制 SHA-256：

```text
control:   efa1c2418cbeecf56b189d9f67be63ccd2c58f9da54a54a8d72aaacceb780368
candidate: e3b17615b620674a812b27a41c639b9a98d68f9746b341e77bc0024153e9359b
```

### 2.2 统计口径

- 正百分比表示**耗时增加**，负百分比表示耗时减少，不是吞吐率变化。
- 对每个有效进程对，计算 `100 × (candidate P50 / control P50 - 1)`，
  再取配对变化的中位数；P95 变化同理。
- 表中的 master/dev P50、P95 是各自进程统计值的跨进程中位数，
  不一定直接复现“配对变化”列。
- P50 变化的 95% 区间来自项目现有统计函数：固定种子、2,000 次 bootstrap，
  重采样单位为独立进程对，不是把所有帧当作独立重复实验。
- P95 是每个进程内 101 帧的 nearest-rank P95，再进行配对；
  不是把多个进程样本混合后的 P95。
- 系统其他负载、GPU 频率和温度没有受控。区间跨零表示方向不确定；
  即使区间不跨零，也不能替代视觉验收或证明具体代码的因果贡献。

统计实现见 [benchmark 报告工具](benches/report.cjs)，场景与测量入口见
[performance benchmark](crates/motion-bench/benches/performance.rs) 和
[artwork](crates/motion-bench/benches/performance/artwork.rs)。

### 2.3 完整性与排除项

- 校验了 504 份成功报告的结构、原始 samples 与汇总一致性。
- 校验冻结二进制身份与硬链接，检查了 2,016 个参考文件的 SHA-256。
- 同一角色、同一场景的重复运行没有发现源码、环境、适配器或输出身份漂移。
- 成功报告记录的进程执行区间不重叠；runner 本身也串行启动测量进程。
- 最终得到 167 个正式测量对、84 个诊断对。

| 排除项 | 原因 | 处理 |
| --- | --- | --- |
| GPU `text` | candidate 预检失败：`Seek replay differs: Gpu/text/17` | 两边都不进入该场景的正式比较，保留失败记录 |
| `ink-long-stroke` 第 5 对 | candidate 整个进程超过 180 秒，没有完整报告 | 排除该候选进程及对应 control；该场景只剩 5 对 |

超时发生在哪个阶段尚未确定。剩余 5 对的 `ink-long-stroke` 统计**以完成为条件**，
不能用其 P95 证明不存在长时间卡顿。没有绕过或放宽任何测试断言。

## 3. GPU 后端端到端结果

单位为 ms。`Δ P50`、`Δ P95` 均为配对耗时变化；区间只列 P50，
P95 的完整区间保存在机器可读结果中。

| 场景 | 有效对数 | master P50 | dev P50 | Δ P50 | P50 95% 区间 | master P95 | dev P95 | Δ P95 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| empty | 6 | 0.556 | 0.477 | -8.4% | -35.1% ~ +14.0% | 0.910 | 0.751 | -14.0% |
| shapes | 6 | 0.858 | 1.174 | +33.9% | +22.4% ~ +104.0% | 1.208 | 1.817 | +55.5% |
| fill | 6 | 0.876 | 1.160 | +33.5% | +23.6% ~ +50.7% | 1.231 | 1.729 | +56.5% |
| stroke | 6 | 4.433 | 6.364 | +44.3% | +33.1% ~ +46.6% | 5.218 | 7.331 | +42.7% |
| ink-fill | 6 | 20.726 | 21.456 | +4.0% | +3.5% ~ +4.9% | 21.584 | 22.913 | +7.2% |
| ink-stroke | 6 | 113.427 | 115.533 | +1.4% | +1.3% ~ +3.5% | 115.363 | 117.932 | +1.3% |
| isolation | 6 | 5.590 | 5.877 | +8.2% | -1.3% ~ +13.9% | 9.200 | 8.569 | -3.3% |
| blur | 6 | 10.240 | 10.613 | +3.0% | +1.5% ~ +5.5% | 12.096 | 12.870 | +7.7% |
| shadow | 6 | 10.973 | 11.306 | +2.9% | +1.8% ~ +4.3% | 12.272 | 12.323 | +2.2% |
| image | 6 | 4.275 | 2.987 | -30.3% | -31.5% ~ -27.6% | 4.943 | 3.761 | -19.3% |
| image-alpha | 6 | 9.286 | 7.420 | -19.0% | -25.0% ~ -14.7% | 12.239 | 10.444 | -14.5% |
| mask-alpha | 6 | 11.911 | 10.725 | -9.9% | -12.8% ~ -8.0% | 16.560 | 14.675 | -10.8% |
| mask-luminance | 6 | 15.457 | 12.276 | -20.3% | -22.8% ~ -15.6% | 19.403 | 16.373 | -15.1% |
| mask-shapes | 6 | 8.270 | 8.356 | +3.1% | -4.5% ~ +10.9% | 12.295 | 12.059 | +1.8% |
| cross-fade | 6 | 19.569 | 16.396 | -16.4% | -17.8% ~ -14.7% | 22.217 | 20.486 | -7.9% |
| wipe | 6 | 32.135 | 29.790 | -7.0% | -7.8% ~ -6.7% | 38.196 | 31.584 | -17.4% |
| push | 6 | 24.128 | 20.482 | -15.0% | -15.9% ~ -13.9% | 28.745 | 24.301 | -14.9% |
| bloom | 6 | 11.304 | 11.456 | +0.7% | +0.3% ~ +3.2% | 11.994 | 12.516 | +2.3% |
| grade | 6 | 7.525 | 7.603 | +1.2% | -11.4% ~ +9.4% | 11.126 | 11.549 | +5.4% |
| motion-blur | 6 | 2.632 | 3.049 | +17.4% | +15.1% ~ +21.0% | 4.902 | 5.362 | +12.7% |
| particles-orbit | 6 | 1.674 | 1.795 | +7.5% | +3.4% ~ +7.9% | 2.178 | 2.158 | +4.0% |
| particles-plexus | 6 | 1.898 | 1.924 | +2.6% | +0.3% ~ +41.5% | 2.246 | 2.327 | +5.4% |
| ink-long-stroke | 5 | 72.402 | 73.430 | +1.3% | +1.1% ~ +1.4% | 72.978 | 74.467 | +0.9% |
| text-parts | 6 | 0.909 | 1.205 | +31.3% | +22.0% ~ +45.0% | 1.204 | 1.705 | +38.7% |
| svg | 6 | 4.591 | 5.444 | +18.0% | +11.2% ~ +20.0% | 5.065 | 6.406 | +20.1% |
| path-sampling | 6 | 1.358 | 1.903 | +43.5% | +32.6% ~ +77.7% | 1.769 | 2.632 | +60.1% |
| path-trim | 6 | 0.776 | 1.188 | +53.4% | +36.7% ~ +106.0% | 1.377 | 1.978 | +83.9% |
| morph-curves | 6 | 0.999 | 1.464 | +48.9% | +26.5% ~ +60.4% | 1.437 | 2.149 | +31.9% |

不能把所有场景的毫秒数简单相加后称为“整体 GPU 性能”，因为没有定义生产场景权重。
上表也不能与早期单轮数据直接比较优化收益：候选身份、进程隔离和测量协议都不完全相同。

## 4. 阶段诊断与归因边界

### 4.1 主机阶段

这些是 **GPU 后端中的 CPU 主机工作**，不是 CPU 后端的复测结果。
每项是 3 次诊断进程中位数的中位数；配对增量另行取中位数，因此不一定等于端点相减。

| 场景 | coverage master→dev | Δ encode | Δ wait/readback | Δ plan |
| --- | --- | --- | --- | --- |
| path-trim | 0.0013→0.0080 | +0.0769 | +0.5038 | -0.0045 |
| morph-curves | 0.0063→0.0257 | +0.1149 | +0.1307 | -0.0075 |
| stroke | 0.6544→0.8079 | +0.2791 | +1.5216 | +0.0277 |
| svg | 1.1749→1.1560 | -0.0054 | -0.2281 | -0.0508 |
| path-sampling | 0.0586→0.1150 | +0.0160 | -0.0661 | -0.0106 |
| image | 0.0195→0.0246 | -0.0772 | -0.0627 | -0.9986 |
| ink-stroke | 0.6779→0.8151 | +2.2905 | -0.0518 | -0.9015 |

`encode` 包含命令构建、上传、提交及 map 设置；`wait/readback` 包含等待 GPU
完成和 CPU 输出提取，不是 GPU 纯拷贝时间。设备执行与主机等待重叠，
**不能把这些数和设备 pass 时间相加，也不能用诊断增量精确分摊正式测量的回退**。

尤其是 `svg`、`path-sampling`：诊断墙钟没有复现正式测量中的净回退。
对它们只能定位新管线内部热点，不能据此认定全部端到端差距来自某个阶段。
`image` 的 plan 约 2.601→1.597 ms，与其加速方向一致，但这仍不是独立消融证据。

### 4.2 dev/gpu 的实际 compute pass

单位为 ms。设备时间取自实际 pass 的开始/结束边界，排除 warmup，并检查单调性。
总计是逐帧求和后的中位数，不保证等于各列中位数相加。
零值表示该 pass 没有执行，不表示整个场景没有 GPU 工作。

| 场景 | flatten | count | prefix | scatter | carry | coverage | 总计 | prefix+carry 占比 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| path-trim | 0.0393 | 0.0140 | 0.0811 | 0.0161 | 0.1774 | 0.0470 | 0.3695 | 70.2% |
| morph-curves | 0.0107 | 0.0081 | 0.1333 | 0.0104 | 0.1108 | 0.0323 | 0.3062 | 78.6% |
| path-sampling | 0.0191 | 0.0079 | 0.0721 | 0.0095 | 0.1106 | 0.0543 | 0.2724 | 67.3% |
| stroke | 0.0000 | 0.0343 | 0.0403 | 0.0496 | 0.8882 | 0.6013 | 1.6240 | 56.7% |
| svg | 0.0823 | 0.0124 | 0.1183 | 0.0186 | 0.0929 | 0.0392 | 0.3618 | 57.7% |
| text-parts | 0.0152 | 0.0119 | 0.0124 | 0.0156 | 0.0935 | 0.0550 | 0.2040 | 52.4% |
| blur | 0.0134 | 0.0114 | 0.0210 | 0.0155 | 0.0602 | 0.0676 | 0.1855 | 42.9% |
| image | 0.0000 | 0.0000 | 0.0000 | 0.0000 | 0.0000 | 0.0000 | 0.0000 | 不适用 |

源码对应关系：

- [coverage shader](crates/motion-render/src/gpu/shaders/coverage.wgsl)：
  `prefix_tiles` 每 fill 一个 invocation 串行遍历 tiles；`carry_rows` 每行串行扫 tile 列和边；
  `signed_area` 每像素扫描所在 tile 的边。
- [device coverage 编码](crates/motion-render/src/gpu/execute/device_coverage.rs)：
  最多六个 pass，workgroups 为零的阶段跳过；每次创建 BindGroup、上传描述数据并清理计数器。
- [设备规划](crates/motion-render/src/gpu/plan/device.rs) 和
  [tile admission](crates/motion-render/src/gpu/raster/device/tiles.rs)：
  在 dispatch 前保守准入 tiles、indices、samples、bytes 和 work。

### 4.3 不可信的原有汇总时间戳

每个分支有 1,764 个正式诊断样本。原有 marker 汇总时间戳中：

| 检查 | master | dev/gpu |
| --- | --- | --- |
| 六个汇总 marker 非单调的样本数 | 1,252 | 1,268 |
| output 的 end 小于 start | 321 | 833 |
| readback-copy 的 end 小于 start | 578 | 785 |

dev/gpu 六阶段实际 pass 内及阶段间没有发现倒序；其中 1,260 个样本执行了设备 coverage。
但有 720 个 coverage pass 的边界落在原汇总 composition 边界之外。
因此不能把两个采样体系硬拼成一条时间线，也不能拿汇总时间减去子阶段。

本次完全不使用这些原汇总 GPU duration 作归因。master 的异常可能表现为环绕后的巨大值，
candidate 的 `saturating_sub` 则可能把异常变成零。不能把零当作“执行免费”。
已确认的是计时数据存在问题，**尚未确认 Metal/驱动层面的根本原因**。
现有实现见 [GPU timing](crates/motion-render/src/gpu/timing.rs)。

### 4.4 视觉与严格可比性

两分支的 artwork 源码、场景配置、编译环境和适配器一致，但 workload 源码指纹不同。
项目官方 `verifyComparable` 的严格检查未通过；本报告没有删除该检查，
也没有将本轮限定同场景比较称为严格可比性验收。

112 张跨分支 GPU RGB 参考帧中有 77 张不同，而且不全是可以直接忽略的微小误差。
例如 `stroke/17` 的整帧 RGB RMSE 约 13.92（0–255），最大通道差 182。
PPM 不包含完整 alpha，尚未按[视觉验收](docs/design/reliability.md#visual-acceptance)
的 Visual 级别（全局 RMS、16×16 分块 RMS、3×3 邻域离群，黑白背景分别比较 alpha 与外观）评估。
整帧 RGB RMSE 与该标准的指标不同，不能互相换算；但 255 中最大 182 的通道差远超
任何邻域余量（最高 8 codes），按该标准应被视为需要定位的结构性差异，而不是容差问题。
这里没有用新标准重新测量这些参考帧，不给出通过或失败的档位结论。

现有视觉契约和测试矩阵见 [开发检查](docs/development.md#independent-backend-acceptance)
与 [GPU coverage 模型](docs/design/rendering.md#coverage-model)。
后续不能通过稀释背景区域、扩大容差、放宽档位或改写 golden 来让性能优化过关；
放宽档位需要说明理由并修改标准页。

## 5. 优化约束与所有权

以下均为**提案**，不是当前已有能力或已验证收益。

- CPU 后端继续拥有自己的 coverage 和数值参考；GPU 优化保持在 `motion-render` 的 GPU 路径内。
- 保持显式设备和资源所有权，不引入全局缓存、隐式时间状态、后台自动选设备或隐式 CPU fallback。
- 新增 scratch、扫描元数据、resident 缓存及 in-flight 引用都先做 bytes/work/device-limit 准入。
  命中缓存不代表可以把语义预算 work 清零；如需改变记账模型，应另立契约并验证边界。
- 不提高当前 benchmark 的 1 GiB / 20-billion-work 额度来掩盖额外成本。
- 不跳过异常传播、finite 检查、取消、设备丢失和成功后发布的边界。
- ABI 改动同时更新 host、plan、WGSL 和
  [layout 断言](crates/motion-render/src/gpu/layout.rs)，保持字段名称、顺序及布局一致。

### 5.1 与 prepared 数据结构的关系

[prepared.rs](crates/motion-render/src/prepared.rs) 定义 CPU `PreparedCpuFrame`
和共享 `FrameRecords`。后者持有 composition records、analytic strips/spans/alphas，
不是 GPU 资源仓库。设备生成的 fill coverage 由
[GPU preparation](crates/motion-render/src/gpu/raster/prepare.rs) 的 `GpuFrame` /
`DeviceGeometry` 及 GPU plan/execution 管理。

因此不应为了缓存方便：

- 给公共 `PreparedCpuFrame` 塞入 GPU Buffer、device、query 或可变缓存。
- 将 `FrameRecords` 的共享语义重新绑定为“GPU 栅格化后的结果”。
- 让 CPU backend 读取 GPU 的私有结果，或者让下层 crate 依赖 facade。

优先在 GPU 内部扩展私有规划、scratch 和设备驻留对象。只有测量证明共享记录格式是瓶颈时，
才单独评估 shared ABI 的变更；不能把当前 tagged 文件本身当成已证明的热点。
所有权约束见 [架构](docs/design/architecture.md)。

## 6. 详细优化方案

### O0 / P0：先建立可信的回归基线

**目标：** 防止优化了错误的瓶颈，或通过改变输出和测量入口获得虚假收益。

1. 将 GPU 六阶段诊断整理为可复用的 opt-in 观察；关闭时不创建 query/readback 资源。
   实际 pass 用开始/结束边界测量，保留 raw ticks、活动阶段和有效性信息。
2. 将“未执行”“设备不支持”“采样异常”“有效零间隔”明确区分。
   倒序、超过包围墙钟或不一致的区间应显式标记为不可用，而不是伪装成零耗时。
   不使用原 marker composition 区间校验新 pass 的包含关系，直到汇总计时本身被验证。
3. 拆分主机指标：descriptor 构建、upload 调用数/字节、pool hit/miss、
   Buffer/BindGroup 创建次数、submit、wait、map/extract。Budget work 不充当实际执行计数。
4. 两分支统一 benchmark 协议和诊断补丁后重新冻结二进制。CPU 后端也按至少 6 对复测；
   旧 CPU 单轮数据只保留历史标签。
5. 保留当前失败记录，调查 `text` replay、`stroke` 大幅像素差和整进程超时；
   本次“遇错跳过”只适用于采集，不等于允许带问题完成优化验收。

**改动位置：** [timing](crates/motion-render/src/gpu/timing.rs)、
[submission](crates/motion-render/src/gpu/execute/submission.rs)、
[性能 benchmark](crates/motion-bench/benches/performance.rs)。

**验收：** 正常/无时间戳支持/未执行阶段/倒序/设备丢失/反向完成都有明确测试；
插桩开关不改变输出和资源契约。严格可比性未满足的比较继续标注原因，不静默绕过。

### O1 / P1：将每 fill 的整型 prefix 改为分段并行扫描

**证据：** `morph-curves` 的 prefix 约 0.1333 ms，`svg` 约 0.1183 ms；
当前每 fill 的一个 invocation 扫完全部 tiles，单 fill 或少量大 fill 并行度低。

**设计步骤：**

1. 按 fill 的 tile 区间切成固定大小 block。优先试验每 workgroup 64/128 lanes 的方案，
   最终大小由设备限制和专项测量确定，不假定所有适配器相同。
2. block 内对 `tiles.count` 做整数 exclusive scan，并输出 block sum。
3. 对同一 fill 的 block sums 做 exclusive scan，再把 block base 加回局部 offset，
   最后加上原 `first_index`。大输入需要有界的分层扫描，不能无限循环等待其他 workgroup。
4. 空 fill、尾部不足一 block、多个 fill 的扫描段必须隔离；保留原 index reservation 约束。
5. 小 fill 可保留现有串行路径，但阈值只依赖显式工作量和设备限制，
   通过独立实验确定，不能依赖播放历史或临时墙钟速度。

**改动位置：** coverage WGSL、GPU pipeline 定义、
[device plan](crates/motion-render/src/gpu/plan/device.rs)、
[coverage dispatch](crates/motion-render/src/gpu/execute/device_coverage.rs)、
tile admission 和 layout。

**风险与约束：** 新增 block scratch 和 dispatch 的成本可能抵消小输入收益。
整数和必须验证溢出和 reservation 上界；不能因“扫描更快”取消现有容量检查。
跨 workgroup 依赖用独立 dispatch/明确同步边界，不把 workgroup barrier 当作全设备 barrier。

**验收：** offsets 与串行参考逐项一致，覆盖 0/1/尾 block/跨 fill/最大准入输入。
计时统计 prefix 的全部新子阶段，而不是只展示其中一个更快的 kernel。

### O2 / P1：拆开 row carry 的边贡献计算与行前缀

**证据：** carry 在 `path-trim` 约 0.1774 ms，在 `stroke` 约 0.8882 ms；
当前每行串行遍历所有 tile 列并扫描相关边。

**设计步骤：**

1. 为每个 `(fill, tile column, pixel row)` 计算独立的 signed-height contribution。
   只在边的最后一个 tile 上计入 `row_height`，保持
   `tile_columns(edge, band, fill.columns).y == column` 的现有语义。
2. 对每个 fill 的每一行沿列做 exclusive scan，得到 tile 左边界的 carry。
   列数较大时采用有界 block scan，不让一个 invocation 扫完整行。
3. tile-local 的 `signed_area` 仍计算该 tile 中边的面积；
   不把尚未走完的斜边提前计入 carry，以免双重累计。
4. 为行 delta、block sums 和 dispatch metadata 更新 bytes/work 准入；
   明确每个输出只有一个 writer，所有读取都对应本次写入。

**风险与约束：** 浮点加法不满足结合律。并行归约会改变求和次序，
不能直接承诺像素不变。需要固定分块/归约顺序，并检查 atomic scatter 带来的边顺序；
如需稳定排序，先预算排序工作与存储。不能默认依赖可移植性不足的浮点原子。

**验收：** 单独覆盖 non-zero/even-odd、相反 winding、完全重复边、孔洞、
离屏左侧边、长斜边、tile seam 和部分尾行。CPU reference/golden 不变。
如果算术变化超过已有契约，停止合入，另行明确输出变化，而不是改宽容差。

### O3 / P1：缓存设备 coverage 结果，而不只是复用工作 Buffer

**证据：** master 有 rasterized coverage cache；当前设备路径会重复执行 coverage。
这对同一几何反复渲染的 warm-frame 尤其不利，但对逐帧变形的收益尚未测定。

**分两步设计：**

1. 先做 GPU 内部、完整几何集合的缓存原型：命中时复用不可变 sample buffer，
   同时省略对应描述数据上传和六阶段计算。先验证收益，不立即改 compositor 的公共协议。
2. 再评估按 fill 缓存，以支持静态背景与动态前景混合。初版可把缓存片段复制到本帧 sample
   atlas，并计入拷贝成本；只有证明必要后才增加逐 fill 的 resident indirection/绑定协议。

**缓存键和所有权：**

- 使用完整几何内容比较防止 hash collision；包含 device-space 变换、局部原点、像素范围、
  fill rule、flatten/tolerance、quality、raster/output/halo 等实际影响结果的输入。
- 绑定设备身份、算法/ABI 版本及相关精度策略；涉及 glyph/SVG 等内容时使用实际几何或不可变版本。
  不以 frame index、节点地址或“上一帧看起来相同”作为完整键。
- coverage 本身与颜色无关时不把 paint color 纳入键，但必须保留路径是否 eligible、
  decal/clip 等域区别。第一版不做平移归一化；以后只能在证明等价的范围内复用。
- 设备 resident 资源由 GPU session/context 管理，独立于 `FrameRecords` 和 CPU cache。
  以现有 resident resource 的 in-flight lease 模式为参考，而不是直接借用 mutable working pool。
- 仅在所属提交成功、设备/finite/output 检查完成后发布缓存。
  pending 引用持有资源，LRU 淘汰不能使未完成帧的存储被改写；设备丢失后清除旧身份。
- 同时约束 entry 数、常驻 bytes、当前请求 peak、查找 work 和 in-flight 存储。
  正常 miss/淘汰与真正分配或设备错误分开，后者不能静默吞掉。

**改动位置：** [GPU preparation](crates/motion-render/src/gpu/raster/prepare.rs)、
device plan、execution，以及 [GPU context](crates/motion-render/src/gpu/context.rs)
中的独立资源所有权。缓存边界必须先形成设计评审，不新增 core→GPU 依赖。

**验收：** cache on/off 结果一致；独立比较冷 miss、100% 静态、混合静态、完全 morph、
随机 seek、预算收紧、淘汰、多个 pending frame 和设备重建。
静态命中必须通过计数证明减少 dispatch/上传；不能只看 hit 数或同一帧 benchmark。

### O4 / P2：减少 tile 内每像素的重复边计算

**证据：** `stroke` 的 coverage pass 约 0.6013 ms；
`signed_area` 在一个 16×16 tile 中，由每个像素重新读取并检查边。

**设计步骤：**

1. 先记录 tile edge-count 分布，区分大部分为空、少量稠密和普遍稠密三类输入。
2. 按有界 chunk 将 tile edge descriptors 放入 workgroup memory，
   由整个 tile 重用，避免每个 invocation 重复读取全部全局 edge 数据。
3. 进一步试验按 pixel row 共享裁剪后的 `y_top/y_bottom/xa/xb`，
   让 16 个列样本重用相同行的边数据；保持原积分、方向和 fill-rule folding。
4. 对空 tile/可证明无局部边的行保留廉价处理，但仍写出 compositor 所需的每个 coverage
   值；不把“没有局部边”误认为“内部 coverage 一定为零”。

**风险与约束：** 当前每 tile 最多准入 4,096 个 edge references，
不能直接把整个最坏列表塞进共享内存；chunk 必须满足设备 workgroup storage 限制。
增加 barrier 后，越界像素 lane 也要参与所有 barrier，只能在最终计算/写入处做有效性判断，
否则尾 tile 可能出现非一致控制流或死锁。保留原误差和 work 上界。

**验收：** 稀疏与稠密场景都测，覆盖非 16 倍数尺寸、guard band、重叠边和 winding。
如果共享内存降低 occupancy 导致总时间变慢，撤回该变体，不只报告带宽减少。

### O5 / P2：以计数为依据优化主机 encode、上传与绑定

**证据：** Buffer pool 已存在，但 BindGroup 不池化，描述数据仍然写入。
`ink-stroke` 没有执行设备 coverage，主机 encode 却增加约 2.2905 ms，
说明不能只优化六阶段 shader 就宣称解决了全部回退。

**设计步骤：**

1. 先区分 pool miss、BindGroup 创建、`queue.write_buffer`、host descriptor 拷贝、
   resource packing 和提交准备的成本，统计实际次数与字节。
2. 复用 CPU scratch；固定长度的 8 个 coverage binding entries 可避免临时 `Vec`。
   这属于低风险小改动，不能预设它足以解释毫秒级差距。
3. 对重复使用的稳定 buffer/layout/range 组合试验有界 BindGroup cache。
   key 必须包含所有 buffer 身份、generation、绑定区间和 layout；
   缓存持有的 buffer 也纳入存储与 in-flight 所有权，不能破坏 pool 回收。
4. 对仍然分散的小上传试验已准入的 arena/staging 聚合，满足 uniform/storage alignment
   和最大 binding/buffer 限制；把 padding、清零、staging 和复制 work 全部计入。
5. 先复用已有批处理能力，仅覆盖测量证明仍有碎片的路径。
   不重复实现已经存在的 vertex/uniform/operation-data 合并。

**改动位置：** context、device coverage execution、texture execution 和对应 plan。
现有能力与限制见 [buffer pooling](docs/design/rendering.md#buffer-pooling-and-residency)
及 [共享上传](docs/design/rendering.md#shared-vertex-uniform-and-operation-data)。

**验收：** encode 时间、创建次数、上传字节、缓存常驻空间同时报告。
特别保护 `image`、`image-alpha` 和 mask/transition 中已有收益；
合并失败不能留下部分预算扣款或半写入的 frame。

### O6 / P3：为纹理消费者和导出验证异步吞吐

**适用范围：** 消费端确实不需要逐帧同步 CPU 像素时，才验证更少 readback 和多帧在途。
当前 `render_frame` 基准要求返回 CPU 可读结果，不能通过删除等待改变它的语义。

1. 复用已有 same-device texture target、pending completion 和 bounded queue，
   不把未完成 GPU 写入的结果提前报告为成功。
2. 为异步场景单独定义吞吐 benchmark，同时保留现有同步端到端 latency benchmark。
   明确输出格式、buffer packing 和消费者，不能跨不同入口宣称速度提升。
3. 对 1/2/3 个在途帧分别测 FPS、端到端 P50/P95、总逻辑 peak、map/staging 压力。
   并发度必须受到显式内存预算限制，不假定越多越快。
4. 保持 frame 顺序、取消、错误归属和资源释放语义；晚完成的失败不能污染已交付帧或缓存。

**验收：** 除性能外，验证输出内容、帧数/顺序、取消、反向完成、设备错误和峰值预算。
这是后续吞吐方向，不是本轮同步延迟回退的直接解释。

## 7. 实施顺序与验收标准

### 7.1 建议拆分

| 顺序 | 独立变更 | 优先验证场景 | 合入前条件 |
| --- | --- | --- | --- |
| 0 | O0：计时有效性、固定基线、视觉问题定位 | text、stroke、empty、超时场景 | 证据可信；失败继续单独记录 |
| 1 | O1：整型 segmented prefix | morph-curves、svg、大范围少 fill | offset 精确一致；包含新增扫描阶段的净收益 |
| 2 | O2：row carry 分解和 scan | path-trim、stroke、path-sampling | 边界/winding/局部区域验收 |
| 3 | O3：GPU coverage 结果缓存 | 静态路径、混合静态、随机 seek | 命中确实减少工作；无生命周期/预算违规 |
| 4 | O4：tile-local 积分复用 | stroke、稠密边、尾 tile | 全适用设备限制内；不扩大误差 |
| 5 | O5：host encode 小改动与上传/绑定复用 | ink-stroke、ink-long-stroke、image | 指标能解释收益，resident 开销受控 |
| 6 | O6：异步纹理/导出吞吐 | 独立定义的消费场景 | 不改变同步 API；有界在途资源 |

O5 的计数与低风险 scratch 改动可在 O0 后独立验证，不必等完所有 shader 优化。
不同优化分开提交、分别做 on/off 消融；最后才组合，检查收益是否叠加及有无互相抵消。

### 7.2 建议的性能目标，不是已测出的收益

- 每项优化先与固定的 `4c38e7d` 候选基线比较，最终再与固定 master 比较。
  shader 算法收益必须先在 cache miss / cache disabled 下验证，避免被新缓存掩盖。
- 主要目标场景的端到端配对 P50 变化中位数以 **不高于 -10%** 为阶段目标，
  且 95% 区间上界小于零；达到这个目标不代表已经追平 master。
- prefix/carry 专项以对应完整阶段耗时减少 **至少 25%** 为探索目标，
  必须统计新增 dispatch、scratch 和上传。kernel 变快但端到端不变，不算完成主目标。
- 静态 cache 实验在 warmup 后，eligible coverage 的命中率目标为 **至少 95%**；
  同时检查真实 dispatch/上传减少。冷 miss、完全动态输入另行报告，不能只报静态收益。
- 对照场景的建议回退预算为 P50 **+3%**、P95 **+5%**：
  它们是工程阈值，不是统计显著性的定义。区间过宽不能算通过，应增加独立进程对。
- 正式比较至少 6 对、每进程至少 101 samples；需要对阶段变化作区间推断时，
  诊断也至少 6 对。两类测量分开进行，不边编译边采集。
- 增加 1920×1080、3840×2160、冷启动、顺序播放和随机 seek；
  本轮 warm-frame 小尺寸结果不外推为所有分辨率或生产吞吐结论。
- CPU 后端必须重新获得固定提交的重复测量结果，不能用附录单轮数据证明“无回退”。

### 7.3 正确性与测试

先运行变更所属模块的最小测试，再扩到以下 GPU 原生矩阵。下面是**后续实施时的建议命令**，
不表示本次报告整理重新执行过这些测试：

```sh
cargo test --locked --offline -p motion-render --all-features --lib gpu:: -- --include-ignored --test-threads=1
```

大场景（文本、SVG、交叉路径与粒子）的原生视觉验收测试目前不存在，
已列入 [开发计划](docs/plan.md#independent-gpu-preparation)；在它落地前，
不能把大分辨率下的性能结果表述为通过了视觉验收。

验收至少覆盖：

- 非零与奇偶 winding、重复/相反边、孔洞、离屏、tile seam、极大坐标、filter halo。
- 按[视觉验收](docs/design/reliability.md#visual-acceptance)分级：CPU reference、
  golden 与 CPU seek replay 为 Exact；GPU 对 CPU 使用 Visual 级别的命名档位
  （Paint、Coverage、Overlap），GPU 自身 seek replay 使用 Replay 档位。
- 每次比较同时满足全局 RMS、16×16 分块 RMS 和 3×3 邻域离群比例，且黑白背景分别比较
  alpha 与外观；两张图必须非空，几何夹具还要证明 GPU 生成了自己的 coverage。
  不用整图背景稀释主体误差，不自动放宽档位；落到更宽的档位需要说明理由并修改标准页。
- O1–O3 的 prefix、row carry 与缓存改动会影响交叉轮廓，须额外跑固定种子随机曲线夹具，
  并报告 Overlap 档的实测余量（本机基线：最大 RMS 0.037、分块 RMS 0.080、离群比例 2.0%）。
- 低 bytes/work 额度、最大 buffer/dispatch/workgroup 限制、cache 淘汰和 pending 生命周期。
- 缓存、并行扫描和 staging 对取消、设备丢失、失败后重试的影响。

测试失败按要求可以保留并跳过后续采集，但必须报告；**未通过正确性验收的方案不能标记为可合入。**
零测试匹配、仅编译成功、某设备不支持而跳过，都不算运行验收。
涉及 ABI、features 或 cfg 的改动还需执行对应结构、layout、feature 和全工作区检查。

## 8. CPU 后端历史单轮结果

本节只保留最初的 CPU 探索结果：640×360、Final、2 warmups、50 samples、
iterations = 1，单轮 master→candidate。`text` 在独立 CPU 运行中补测。
候选版本不能可靠地统一标注为本轮 `4c38e7d`，因此**本节不是当前固定提交的 CPU 验收**。
不计算独立进程置信区间，不宣称统计显著性，也不使用这 50 samples 作可靠 P95 结论。

单位为 ms；变化直接由单轮 P50 端点计算。

| 场景 | 历史 master P50 | 历史 candidate P50 | 耗时变化 |
| --- | --- | --- | --- |
| empty | 0.028 | 0.027 | -2.7% |
| shapes | 0.715 | 0.720 | +0.7% |
| fill | 0.626 | 0.611 | -2.4% |
| stroke | 3.776 | 3.893 | +3.1% |
| ink-fill | 23.714 | 23.239 | -2.0% |
| ink-stroke | 113.734 | 112.692 | -0.9% |
| isolation | 1.207 | 1.206 | -0.1% |
| blur | 5.915 | 5.926 | +0.2% |
| shadow | 8.822 | 8.684 | -1.6% |
| image | 10.821 | 9.694 | -10.4% |
| image-alpha | 12.193 | 11.412 | -6.4% |
| mask-alpha | 10.586 | 10.517 | -0.6% |
| mask-luminance | 18.751 | 18.691 | -0.3% |
| mask-shapes | 1.702 | 1.655 | -2.7% |
| cross-fade | 22.134 | 21.822 | -1.4% |
| wipe | 32.080 | 31.771 | -1.0% |
| push | 14.637 | 14.532 | -0.7% |
| bloom | 9.310 | 9.343 | +0.4% |
| grade | 2.639 | 2.674 | +1.3% |
| motion-blur | 5.103 | 5.073 | -0.6% |
| particles-orbit | 1.358 | 1.545 | +13.7% |
| particles-plexus | 2.607 | 2.750 | +5.5% |
| ink-long-stroke | 71.016 | 70.264 | -1.1% |
| text-parts | 0.573 | 0.569 | -0.7% |
| svg | 3.827 | 3.747 | -2.1% |
| path-sampling | 0.979 | 1.013 | +3.4% |
| path-trim | 0.429 | 0.424 | -1.1% |
| morph-curves | 0.751 | 0.737 | -1.8% |
| text | 2.772 | 2.933 | +5.8% |

只能描述为：29 个场景中，22 个单轮变化落在 ±3% 内；跨场景变化的中位数约 -0.75%。
这是未加权的描述统计，不是 CPU 后端“整体提升 0.75%”，更不证明其余变化可归因于本分支。

## 9. 证据位置与复现注意事项

本机本轮完整证据位于：

```text
~/.copilot/session-state/66cfe20f-1b83-4004-a0dc-4d3544a77936/files/gpu-investigation/
  state.json
  summary.json
  comparison.csv
  report.html
  build-validation.json
  run.cjs
  analyze.cjs
  candidate-instrumentation.patch
  control-instrumentation.patch
  bin/control
  bin/candidate
  measurement/
  diagnostics/
```

旧 CPU 报告位于本机 `target/performance/` 的 `control`、`candidate`、
`control-text-cpu`、`candidate-text-cpu` 子目录。这些本机证据不随本文自动提交，
但本文的数据表、协议、失败项和结论是自包含的。

构建使用 `CARGO_INCREMENTAL=0 cargo bench --locked --offline -j 2 -p motion-bench
--features gpu --bench performance --no-run`。control 使用原 target，candidate 使用独立 target；
后者已清理。复现时应给两个工作树分别指定 `CARGO_TARGET_DIR`，
不要依靠跨工作树共享 target 的旧产物判断提交身份。

当时诊断工作树中的 `gpu::timing::tests` 已实际执行：3 passed，0 failed，
包括 native timestamp plumbing 测试；这不能代替按[视觉验收](docs/design/reliability.md#visual-acceptance)
分级执行的 GPU 视觉验收。
诊断开关 `MOTION_PROFILE_STAGES=1` 只存在于归档补丁，不是当前生产 benchmark 的默认能力；
不要与已有 breadth benchmark 的 `MOTION_BENCH_TIMINGS` 混淆。
当时报告整理没有重新运行完整工作区测试、增加依赖或执行新一轮性能采集。

### 本次报告校验

- 28 行 GPU 数据逐项与归档 `summary.json` 核对，通过。
- 29 行历史 CPU 数据逐项与四份原始报告核对，通过；四份报告的 schema/sample 校验通过。
- 本文相对源码/文档链接和所引用 heading anchors 的专项检查通过。
  根目录 `per.md` 不在现有全仓文档扫描 roots 内，因此没有仅靠全仓命令代替本文检查。
- `git diff --check` 通过。
- `node tools/check-docs.cjs` 未通过：现有 [CLI 文档](docs/cli.md) 第 229 行引用
  `../.github/skills/motion-video/SKILL.md`，目标在当前工作树不存在。
  已确认 HEAD 中原有该引用，且本次未修改该文档；保留这个既有问题，没有进行无关修复。

## 10. O0 进展与新一轮 CPU/GPU 基线

### 10.1 状态和测量边界

- 已实现可选 GPU 时间戳：六个实际 coverage pass 各有 raw ticks、耗时和有效性状态；
  关闭或设备不支持时不创建 query/readback。区分未执行、有效零、倒序、乱序、
  超过包围墙钟、非法周期和无法表示的结果。benchmark 与 Studio 展示状态而不是把异常当成零。
- 在本机 Apple M1 Max / Metal 上，实际执行了 timestamp plumbing、coverage 插桩前后输出与
  预算相等、阶段计时输出保持、异步句柄反向完成和失败隔离测试；均通过。之后又在同一
  适配器上验证了禁用 timestamp 特性的独立设备，以及主动销毁专用设备的在途失败处理。
  物理上不支持 timestamp 的适配器和非主动设备丢失仍未验证。
- 主机 descriptor、upload 字节/调用、pool 命中、Buffer/BindGroup 创建、submit、
  wait 与 map/extract 的独立计数尚未实现；`text` replay、`stroke` 跨分支差异和此前
  180 秒超时的根因尚未查明。O0 因此是**部分完成**，不进入优化验收。
- 当时的 `text` 预检使用同一 session，依次渲染目标帧、另一帧、目标帧并逐字节比较 RGB；
  frame 17 失败并不单独证明缓存错误。一个值得实验的假设是
  [coverage shader](crates/motion-render/src/gpu/shaders/coverage.wgsl) 使用原子操作分配
  edge 索引位置，随后按该顺序累加浮点面积；线程执行次序不同可能影响舍入。
  后续消融与差异见第 11 节；这仍不是已完整证明的根因。
- 当前 `GpuTimings` 的三个 aggregate 字段由 `Duration` 改为带状态的 `GpuTimestamp`，
  属于公开 API 的破坏性变化，合入前需单独审查兼容性及迁移方式。

本轮 control 是 `master@6c89650afdba745c29ffac811610dbe901d42e9b` 加先前的诊断补丁；
candidate 是 `b52e9eb51816e013fb3b6bc17b2968b3abfb02b0` 加构建时完整已跟踪脏树补丁，
包含其他已有改动，**不能标为纯提交或当前 HEAD**。两侧诊断补丁不同；正式测量均关闭插桩。
嵌入二进制的 artwork 指纹相同（`823893673e1c7cae`），其他 workload 指纹不同，
官方严格检查以配置不兼容拒绝全部 57 个场景；以下为限定场景的条件性比较，不是验收。

```text
control SHA-256:   efa1c2418cbeecf56b189d9f67be63ccd2c58f9da54a54a8d72aaacceb780368
candidate SHA-256: b7db66959d319602fe71e13845701e986775472bcb6b64feed45efcb63608578
```

使用 640×360 Final、每场景独立进程、2 次 warmup、101 samples、6 对 AB/BA，
每进程最多 180 秒；统计口径与第 2.2 节相同。报告器验证了 687 份成功报告、
687 个冻结可执行文件硬链接，并计算了 2,748 个参考文件的 SHA-256；
同角色重复运行身份一致，
成功进程执行区间不重叠。GPU `text` 候选预检在 frame 17 再次失败，
两侧该场景排除；CPU `text` 正常。其余 342 对全部完成，没有本轮超时。
这不能证明此前 `ink-long-stroke` 卡顿原因已解决。

CPU 的 116 张跨分支 RGB 参考帧相同；GPU 的 112 张有 77 张不同。
PPM 不包含完整 alpha，且严格可比性仍未通过。六对区间未做多重比较校正，
系统负载、频率和温度未受控；正值表示耗时增加而非吞吐增加。
CPU 16 个场景 P50 区间高于零、0 个低于零、13 个跨零；GPU 分别为 19、7、2 个。
小幅变化不能直接归因于特定代码或解释为整体速度变化。

### 10.2 CPU 六对结果

单位 ms；端点为各角色进程统计的中位数，百分比为配对变化中位数；
区间为 P50 变化的 95% bootstrap 区间。所有场景均为六对。

| 场景 | master P50 | candidate P50 | Δ P50 | P50 95% 区间 | master P95 | candidate P95 | Δ P95 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| text | 2.482 | 2.679 | +8.2% | +7.8% ~ +8.9% | 2.604 | 2.798 | +7.6% |
| empty | 0.027 | 0.027 | +0.6% | -1.6% ~ +4.5% | 0.029 | 0.029 | -1.0% |
| shapes | 0.704 | 0.714 | +1.2% | -0.1% ~ +2.8% | 0.731 | 0.734 | +1.8% |
| fill | 0.599 | 0.603 | +0.6% | +0.2% ~ +0.9% | 0.616 | 0.621 | +0.5% |
| stroke | 3.631 | 3.720 | +2.6% | +1.8% ~ +3.2% | 3.747 | 3.843 | +2.4% |
| ink-fill | 22.696 | 22.622 | -0.0% | -0.9% ~ +0.2% | 23.173 | 23.250 | +0.0% |
| ink-stroke | 109.997 | 110.132 | +0.1% | -1.3% ~ +1.1% | 110.795 | 111.012 | +0.2% |
| isolation | 1.122 | 1.141 | +1.7% | +1.5% ~ +2.3% | 1.154 | 1.173 | +2.6% |
| blur | 5.698 | 5.713 | +0.3% | +0.2% ~ +0.5% | 5.755 | 5.801 | +0.8% |
| shadow | 8.312 | 8.327 | +0.1% | -0.3% ~ +0.3% | 8.380 | 8.406 | +0.2% |
| image | 9.379 | 9.427 | +0.5% | +0.1% ~ +0.7% | 9.576 | 9.573 | +0.3% |
| image-alpha | 11.008 | 10.980 | -0.2% | -0.4% ~ +1.0% | 11.092 | 11.060 | -0.2% |
| mask-alpha | 9.945 | 9.986 | +0.4% | -0.4% ~ +0.7% | 10.022 | 10.175 | +0.5% |
| mask-luminance | 18.078 | 18.186 | +0.5% | +0.3% ~ +1.6% | 18.384 | 18.820 | +1.3% |
| mask-shapes | 1.522 | 1.550 | +1.9% | +1.5% ~ +2.6% | 1.617 | 1.598 | +2.0% |
| cross-fade | 21.058 | 21.051 | +0.3% | -0.2% ~ +0.9% | 21.388 | 21.470 | +0.2% |
| wipe | 30.555 | 30.527 | +0.4% | -1.1% ~ +1.0% | 31.122 | 31.349 | +1.1% |
| push | 14.149 | 14.061 | +0.4% | -0.8% ~ +0.5% | 14.363 | 14.335 | +0.0% |
| bloom | 8.906 | 8.925 | +0.4% | +0.2% ~ +0.6% | 9.109 | 9.135 | +0.3% |
| grade | 2.507 | 2.522 | +0.7% | +0.2% ~ +1.5% | 2.570 | 2.606 | +0.7% |
| motion-blur | 4.765 | 4.806 | +0.7% | +0.6% ~ +1.1% | 4.910 | 4.894 | +0.6% |
| particles-orbit | 1.310 | 1.353 | +3.6% | +2.1% ~ +5.3% | 1.339 | 1.385 | +3.8% |
| particles-plexus | 2.470 | 2.547 | +3.0% | +0.4% ~ +3.4% | 2.532 | 2.614 | +2.8% |
| ink-long-stroke | 69.242 | 68.981 | +0.1% | -0.4% ~ +0.3% | 69.817 | 70.322 | +0.1% |
| text-parts | 0.527 | 0.534 | +1.4% | +1.2% ~ +2.6% | 0.543 | 0.556 | +1.8% |
| svg | 3.608 | 3.616 | +0.4% | +0.1% ~ +0.8% | 3.656 | 3.666 | +0.5% |
| path-sampling | 0.778 | 0.803 | +2.9% | +2.7% ~ +5.6% | 0.803 | 0.833 | +3.6% |
| path-trim | 0.415 | 0.414 | -0.1% | -0.8% ~ +1.6% | 0.434 | 0.423 | -0.8% |
| morph-curves | 0.722 | 0.717 | -0.4% | -2.6% ~ +0.6% | 0.739 | 0.744 | +0.0% |

### 10.3 GPU 六对结果

单位和统计口径同上；所有列出的场景均为六对，失败的 `text` 不在表内。

| 场景 | master P50 | candidate P50 | Δ P50 | P50 95% 区间 | master P95 | candidate P95 | Δ P95 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| empty | 0.493 | 0.466 | -2.2% | -32.5% ~ +21.6% | 0.694 | 0.636 | -6.9% |
| shapes | 0.799 | 1.087 | +32.9% | +21.3% ~ +38.0% | 1.054 | 1.463 | +35.1% |
| fill | 0.773 | 0.954 | +26.3% | +17.6% ~ +30.6% | 0.995 | 1.226 | +23.1% |
| stroke | 4.237 | 5.147 | +21.5% | +20.8% ~ +23.5% | 4.334 | 6.431 | +49.1% |
| ink-fill | 20.148 | 20.765 | +3.0% | +2.9% ~ +3.3% | 20.436 | 20.985 | +3.0% |
| ink-stroke | 111.629 | 112.958 | +1.2% | +1.0% ~ +1.5% | 112.521 | 113.576 | +0.9% |
| isolation | 4.521 | 4.780 | +5.6% | +5.2% ~ +6.3% | 7.615 | 7.318 | -3.9% |
| blur | 8.750 | 8.879 | +2.7% | +0.0% ~ +4.5% | 11.056 | 11.879 | +5.8% |
| shadow | 10.679 | 10.978 | +2.2% | +1.7% ~ +4.1% | 12.224 | 12.536 | +3.6% |
| image | 4.039 | 2.684 | -33.6% | -34.1% ~ -33.5% | 4.122 | 2.772 | -33.0% |
| image-alpha | 7.669 | 6.133 | -20.1% | -20.7% ~ -17.3% | 11.202 | 9.432 | -15.7% |
| mask-alpha | 9.464 | 8.167 | -13.8% | -17.7% ~ -10.4% | 14.077 | 13.165 | -6.3% |
| mask-luminance | 13.372 | 9.608 | -28.8% | -29.2% ~ -25.5% | 18.129 | 15.252 | -16.4% |
| mask-shapes | 6.298 | 6.666 | +5.7% | +5.2% ~ +6.3% | 9.792 | 10.432 | +5.3% |
| cross-fade | 18.088 | 14.773 | -18.5% | -20.0% ~ -17.2% | 21.834 | 19.179 | -13.1% |
| wipe | 28.841 | 27.255 | -5.5% | -6.1% ~ -5.0% | 31.158 | 29.598 | -4.4% |
| push | 22.886 | 20.384 | -10.8% | -12.0% ~ -10.0% | 27.013 | 22.402 | -16.8% |
| bloom | 11.105 | 11.151 | +0.7% | +0.4% ~ +1.1% | 11.369 | 11.767 | +3.5% |
| grade | 5.543 | 5.822 | +5.1% | +4.4% ~ +5.9% | 9.517 | 10.484 | +7.1% |
| motion-blur | 2.312 | 2.597 | +12.9% | +12.1% ~ +13.9% | 4.085 | 4.647 | +14.1% |
| particles-orbit | 1.635 | 1.755 | +7.4% | +6.4% ~ +8.7% | 1.802 | 1.836 | +0.4% |
| particles-plexus | 1.795 | 1.804 | -0.8% | -4.4% ~ +2.5% | 2.202 | 2.175 | -3.6% |
| ink-long-stroke | 71.001 | 71.848 | +1.2% | +1.0% ~ +1.3% | 71.439 | 72.427 | +1.0% |
| text-parts | 0.834 | 1.043 | +25.2% | +14.6% ~ +34.3% | 0.997 | 1.426 | +42.9% |
| svg | 3.845 | 4.158 | +9.6% | +5.6% ~ +10.7% | 4.501 | 5.498 | +21.6% |
| path-sampling | 1.120 | 1.578 | +40.9% | +38.9% ~ +43.1% | 1.312 | 1.953 | +46.3% |
| path-trim | 0.623 | 0.984 | +57.6% | +43.4% ~ +62.8% | 0.864 | 1.314 | +53.1% |
| morph-curves | 0.947 | 1.197 | +30.5% | +20.1% ~ +48.9% | 1.140 | 1.359 | +18.7% |

新一轮原始数据、冻结二进制、完整 candidate 补丁、复核脚本及机器可读结果保存在
第 9 节所列 session 目录的 `o0-baseline-v2/`、`bin/o0-candidate`、
`o0-after-build.patch` 和 `o0-analysis.cjs`。最初因 control 编译时嵌入的 manifest
目录被清理而失败的尝试保存在 `o0-baseline/`，没有混入上述结果。
恢复空目录仅用于解析已编译路径，并未更改源码或二进制。

本轮针对当前工作树重新运行时间戳单元测试：3 passed、1 个 native 测试按常规忽略；
另按名称串行执行四项 native GPU 测试，均为 1 passed。`motion-render`、`motion-bench`、
`motion-studio` 的相关 clippy 检查、结构与 expect 检查，以及文档检查均通过。
之前文档检查的缺失链接已在当前工作树修复。全仓格式检查现在可以启动，
但因其他已有文件的 import 排序不符合 rustfmt 失败；没有全仓写格式或更改这些文件。
这一轮没有运行完整工作区测试；后续设备测试见第 11 节。

## 11. GPU 回归与设备故障补充调查

- 在 Apple M1 Max / Metal 上，`native_timestamp_disabled_device_preserves_output`
  使用不启用 timestamp 特性的独立逻辑设备，验证开关计时前后输出及统计一致，
  且不返回 timestamp 句柄；测试通过。这不是物理上不支持该特性的设备。
- `native_device_loss_does_not_publish_pending_timestamps` 在提交后主动销毁专用设备，
  验证在途句柄不发布结果、返回 `DeviceLost`，后续渲染继续报设备丢失；测试通过。
  此测试不能替代驱动崩溃、拔出设备等非主动丢失的验证。
- 正常 shader 下的 GPU `text` 严格 seek replay 在 frame 17 仍失败，独立回归测试
  本次有 2 个像素的 packed RGB 不同；先前 benchmark 采样中有 3 个 RGB 通道不同，
  最大差 1。两者计数单位不同，不能直接比较。未放宽逐像素相等要求。
  暂时将 tile edge scatter 串行化后，两个独立 benchmark 进程通过完整 replay，
  但耗时约 31 ms；这支持浮点累加顺序假设，并不证明唯一原因。
  实验改动已撤回，不能把未计入相应 work 预算的串行方案作为修复。
- `stroke` 的 master GPU 与 candidate CPU 四张 RGB 参考图一致；candidate GPU
  的 frame 17 相对 CPU 整图 RGB RMSE 为 13.924/255，最大通道差 182。
  当前工作树的 CPU/GPU 图复现差异，定位到候选 GPU 路径，但具体算法原因仍未知。
  将所有 path 暂时导向 CPU 几何准备的额外实验被 GPU 自检拒绝，未产生有效对照；
  实验改动已撤回，不能据此推断原因。
- 历史 `ink-long-stroke` 超时前四张预检 PPM 已写出，正式报告未生成；
  现有证据不能区分 warmup、测量、GPU 等待或收尾。后续完成不证明根因已修复。
- 主机 per-frame 资源、上传、提交和等待等完整细分计数仍未实现。共享 context 与
  多个在途 frame 需要隔离的计数生命周期，不能将共享全局计数当作单帧结果。

本节的设备测试命令：

```sh
cargo test --locked --offline -p motion-render --features gpu --lib native_timestamp_disabled_device_preserves_output -- --ignored --nocapture --test-threads=1
cargo test --locked --offline -p motion-render --features gpu --lib native_device_loss_does_not_publish_pending_timestamps -- --ignored --nocapture --test-threads=1
```

两项均为 1 passed。历史上的逐字节测试曾失败；按用户要求，该测试已删除，
GPU benchmark 预检改用现行 Replay 视觉档位，CPU benchmark 保持精确比较。
这一变更不改变或追认历史测量的结果。

## 12. 按现行视觉标准复测 text 与 stroke

测试使用当前工作树、Apple M1 Max / Metal、640×360 Final，场景和帧序列
`[0, 45, 17, 89]` 来自 benchmark。字体为
`tests/fixtures/fonts/NotoSans-Regular.ttf`，SHA-256 为
`b85c38ecea8a7cfb39c24e395a4007474fa5a4fc864f6ee33309eb4948d232d5`。
测试直接复用渲染器的视觉比较器，对透明 straight-alpha RGBA8 检查 alpha 和黑白背景，
并断言 GPU 几何确实经过设备 coverage；未改变视觉标准。
结果只适用于这台设备、这些帧和当前工作树。

| 场景和检查 | 结果 | 实测 |
| --- | --- | --- |
| `text` GPU seek，Replay 档 | 通过 | 独立进程重复 3 次；透明和不透明输出共 24 次比较全部通过。最大 RMS 0.00001513，最大 tile RMS 0.00022692，outlier 均为 0 |
| `text` GPU seek，历史逐字节检查 | 失败 | frame 17 出现少量不同像素；按用户要求该检查已删除 |
| `text` GPU 对 CPU，Coverage 档 | 通过 | 四帧透明输出的 RMS、tile RMS 和 outlier 均为 0；断言有非零的抗锯齿 alpha |
| `stroke` GPU 对 CPU，Overlap 档 | 失败 | 四帧全局 RMS 分别为 0.111998、0.111610、0.112602、0.112012，均超过 0.05；最差 tile RMS 为 0.262919–0.376854，均超过 0.12 |

`stroke` 的 BrushStroke 几何由相互重叠的笔尖多边形和连接多边形构成，
因此使用 Overlap 档。其四帧 outlier 分别为 84、60、48、36，
但不能用较低的 outlier 数掩盖全局和局部 RMS 失败。

额外构造两个完全重合、同方向、亚像素位置的矩形轮廓：CPU 对单个和重复轮廓
输出完全一致；GPU 的重复轮廓在角像素输出 alpha 255，而 CPU 为 143。
该最小夹具的 RMS 为 0.067664，超过 Overlap 的 0.05，测试失败。
这验证当前 GPU 的重叠覆盖处理存在独立缺陷，也与 `stroke` 结果一致；
但单个消融不能证明复杂笔迹的全部误差均由这一机制造成。
现有 shader 先累加有符号面积，再对 non-zero winding 截断；该近似不能一般性地
还原像素内部的重叠区域。修复需要有界处理像素内 winding/交叠，并更新 work/bytes
准入及跨后端测试；不能用扩大容差、隐式 CPU fallback 或仅特判重复轮廓代替。
本轮未修改渲染算法，因此 `stroke` 尚未修复。

测试入口：

```sh
MOTION_TEST_FONT=/absolute/path/to/NotoSans-Regular.ttf cargo test --locked --offline -p motion-bench --features gpu --test performance_replay gpu_text_seek_replay_meets_visual_tier -- --ignored --nocapture --test-threads=1
MOTION_TEST_FONT=/absolute/path/to/NotoSans-Regular.ttf cargo test --locked --offline -p motion-bench --features gpu --test performance_replay gpu_text_matches_cpu_visual_coverage_tier -- --ignored --nocapture --test-threads=1
cargo test --locked --offline -p motion-bench --features gpu --test performance_replay gpu_stroke_matches_cpu_visual_overlap_tier -- --ignored --nocapture --test-threads=1
cargo test --locked --offline -p motion-render --features gpu --lib duplicate_contour_matches_cpu_visual_overlap_tier -- --ignored --nocapture --test-threads=1
```

前两项通过；后两项为当时的失败记录，保留为显式 native 回归。

## 13. O1 整型 segmented prefix 实施与测量

O1 使用 64 tile 的分段扫描、按 fill 扫描 block sum、再加 block base 的三个独立
dispatch。block scratch 复用 tile 的保留字；扫描计入额外 work，单 tile fill 也计入
一个完整 workgroup 的固定成本。因此在相同 work 上限下，以前接近上限的输入可能
不再被准入。prefix 时间戳包围三个 dispatch，而不是只测第一个。

GPU 原生 offset 回读与串行参考逐项相等：测试覆盖零计数、跨 fill、尾块、多批
block sum、二维 dispatch、1,048,576 个 tile 和接近 `u32::MAX` 的 index reservation。
空场景不生成 fill 或扫描 dispatch。coverage shader 验证、预算边界、CPU/GPU
coverage 与 replay、tile seam/孔洞及时间戳边界测试均通过。

以下是 Apple M1 Max / Metal、Final、GPU 同步 warm-frame 的探索性测量。control 和
candidate 是包含相同 benchmark 源码的隔离快照，唯一的运行时代码差异为 O1；
两组 workload fingerprint 均为 `884561cd0fd6d8ea`。每对参考图的四帧 RGB
逐字节一致。百分比是每对进程 candidate P50 相对 control 的变化，再取中位数：

| 场景 | 分辨率 | 独立进程对 × 每进程样本 | 配对 P50 变化中位数 |
| --- | --- | --- | --- |
| morph-curves | 640×360 | 6 × 101 | +0.53% |
| svg | 640×360 | 6 × 101 | -0.82% |
| morph-curves | 1920×1080 | 3 × 21 | -19.66% |

大图三对分别为 -19.66%、-16.39%、-57.30%；最后一对的 control 明显偏慢，且
candidate 的 P95 在这些对中高于 control。系统负载、GPU 频率和温度未受控，
样本不足以证明稳定端到端净收益或 P95 改善；640×360 的变化也不应解释为确定收益。
三对独立诊断运行的 prefix 中位数分别为：morph-curves control
296.375/114.208/113.792 µs、candidate 47.125/79.250/77.458 µs；svg control
262.792/263.209/262.417 µs、candidate 85.416/81.209/80.541 µs。这些
candidate 区间包含所有新增扫描阶段，但插桩测量不能代替上述端到端结果。

原始报告和冻结二进制位于本次 session 的 `files/o1-final-*`、`files/o1-large-*`、
`files/o1-final-diag-*`、`files/o1-control-frozen` 和 `files/o1-candidate-final`。
不同 workload fingerprint 的早期试跑没有纳入以上比较。全仓格式检查仍因既有的
`cpu.rs` 和 `cpu/coverage_cache.rs` import 排序失败，未修改这些无关文件；
相关文件格式、结构、expect、文档和 `motion-render` Clippy 检查通过。
