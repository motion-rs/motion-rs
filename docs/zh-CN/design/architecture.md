# 系统架构设计

Motion 的渲染流水线为：`SceneContext -> Scene -> Node -> ValidatedDisplayList -> FrameIr -> 后端准备 -> CPU/GPU -> 输出`。CPU 准备阶段生成 `PreparedCpuFrame`；GPU 准备阶段生成其专属的几何与合成记录。Scene 是确定性的显式时间与固定资源的纯函数，而非由回放时钟推进的可变对象。

## 求值与所有权模型

SceneContext 提供有符号的局部/全局 ticks、有理数帧率、逻辑包围盒、共享的 `Evaluation`（包含质量模式、资源库、诊断接收器）、带键的随机数流以及有界缓存。Node 快照是完全不可变的写时复制结构。编译阶段负责计算排版布局、解析定位、将 `DeferredContent` 展开为普通节点、记录 Drawable 指令并校验所有分支（包括不可见内容）。

Node 编译在录制指令的同时进行验证，并直接返回 `ValidatedDisplayList`。`DisplayList` 属于可直接编辑的底层 API。`ValidatedDisplayList` 冻结一份经验证的 `Arc` 快照，并保留结构化约束信息以供后续配额检查；但这并不代表显卡硬件必然可用或光栅化预算一定充足。图片画刷引用不可变像素数据；已准备帧不存在延迟图像生成阶段。

内部的 FrameIr 在分配具体的覆盖率内存前，先行规范化有序作用域、画刷与裁剪引用、滤镜域、带内边距的尺寸以及输出视口窗口。它保留 f64 设备空间曲线、保守的 Wang 二阶导数线段上界以及已准入的资源字节，不包含光栅覆盖率。局部几何配方保留了 CPU 变换矩阵算术以及覆盖率缓存标识。两个后端的准备逻辑各自独立消费这一份不可变描述，而不相互依赖对方的产物。CPU 准备阶段直接生成紧凑覆盖率；GPU 准备阶段独立在设备端展平曲线以生成 f32 覆盖率，并在遇到不符合条件的复杂路径或裁剪时保留显式的 CPU 覆盖率。规范化阶段的存储与计算开销计入准备统计信息中；活跃的 IR 内存会与后端准备阶段的暂存区重叠，从而增加其峰值准入开销。两个渲染器在保持绘制指令语义与资源契约一致的前提下，不强求生成逐位完全相同的覆盖率数值。输出形态包括强类型 `FrameBuffer`（透明 PNG 使用直通 RGBA8）、借用的外部 `RenderTarget`（纯白背景的不透明帧为直通 RGBA8 FrameBuffer）或保序的 `Encoder`。

RenderOptions 存储全局策略；RenderSession 拥有可复用的多级缓存、后端运行状态与请求诊断。FrameRequest 通过索引或精确时间寻址一帧，并提供有理数帧率、时长、背景色、目标表面与像素格式。FrameRenderer 将渲染会话与 Composition 的帧索引时钟绑定。预览系统可合并丢弃冗余请求，而导出系统则严格渲染每个被选中的帧，二者无需定义各自孤立的场景/渲染流水线。

代码实现的 Package 归属划分见[包与特性划分](#包与特性管理)。运行时各模块按职责清晰隔离。单元测试与所属模块就近存放，规模较大时拆分到相邻的 `tests.rs` 文件中；跨包集成测试统一置于 facade crate 的 `tests/` 目录下。仅有 facade 负责组装面向用户的公开创作 API。引擎整体严禁使用 `unsafe` 代码；仅第一方的 FFmpeg FFI 绑定 crate 局部管理编解码相关的 unsafe 操作。

GPU 执行流水线细分为 `execute/submission`、`execute/textures`、`execute/particles`、`execute/pending` 与 `execute/queue`：将命令提交/像素打包、纹理计算与保序混合、粒子分箱、任务完成/回收以及有界保序调度解耦为独立可读的子模块。入口模块仅负责连接这些子模块并声明测试。粒子样式、分批/索引与渲染流程保持隔离，协议与光栅化测试置于 `particles/render/`。代码结构检查脚本将这些实现文件严格限制在 800 行以内；纯测试文件不受此限制。属性测试与原生固件测试见[开发指南](../development.md)。Criterion 工作负载与基准命令见[性能基准](../benchmarks.md#cpu-criterion-帧测试)。

## 数值精度与资源管理

时间计算全程使用整数 ticks 与有理数帧率。创作层决策采用 f64 双精度浮点；工作像素为有限的线性 sRGB 预乘 f32。图片 Mipmap 采用 RGBA16F。所有 GPU 内部工作颜色纹理均使用池化的 RGBA32F 纹理，包括遮罩与保存的滤镜源图；外部 RGBA16F 输出仅在其声明的边界处执行量化。仅最终的工作图像会 resolve 到缓冲区中用于线性回读；打包输出与外部纹理输出直接读取该工作纹理，避免分配全量线性浮点缓冲区。粒子元数据与覆盖率/画刷上传依然保留为 Buffer 形式。CPU 确定性重放保持不变；GPU 输出与 CPU 之间根据[渲染契约](rendering.md)中按测试固件划分的容差级别检查视觉等价性，而非逐比特一致，包含在同一显卡与驱动下的重放检验。

CPU 内存输出渲染全宽度的自适应分带区域，并带有保守的滤镜依赖光晕（Halo）。准备阶段会缓存嵌套滤镜链的最大光晕；并列的兄弟滤镜不会盲目叠加它们的光晕。分带区域通常为 16 行，随光晕动态扩展，在内存预算允许时可直接使用全高线性中间缓冲区。当字节预算紧张时，流水线会收缩分带高度并重试，但已消耗的计算量不予退还。具体策略详见[渲染与导出](rendering.md#执行与输出路径)。仅有有效区域会被执行色彩转换；最终编码平面的内存持有至输出 `Drop`，随后退还至单独有界的共享池中。图层/遮罩/滤镜的暂存空间使用会话级像素池。计算工作量在各分带间累加统计；逻辑峰值内存涵盖最终输出以及同时存在的准备、捕获与转换有效负载。显式的线性帧与不透明帧入口保持可用，它们不是隐式缓存。

画刷准备保留渐变几何结构与图案反向变换。逐行采样复用垂直项并以精确有界的二进制网格步进递推；遇到不受支持的精度要求时退化为直接标量映射。对于普通路径与文字字形轮廓，内部覆盖率采用规范的设备原点坐标，在缓存键中完备保留小数值相位与完整几何等价性。裁剪与超出安全范围的大坐标使用原始原始键。

`Canvas::instances` 记录一个 `InstanceBatch`；Rust 负责计算所有运动轨迹与精灵图对齐数学。可选的经过校验的 `InstanceUpload` 包含通用的 48 字节图元记录，而非 GPU 专有程序或未处理的物理模拟原始状态。消费端在显卡执行前校验实例数量与精确字节开销。GPU 辅助/tile 元数据属于独立存储，分开进行预算控制。

结构化配额统计的是出现次数（Occurrences），而非唯一共享指针数量。光栅化配额统计已准入的逻辑计算工作量与活跃负载，而非内存分配器总容量、显卡驱动显存、任意用户回调或系统进程 RSS。所有持久保留的缓存必须具备独立的条目数与字节上限。资源的修改会创建全新的快照标识。

`Assets` 在同一个 `ResourceStore` 内统一管理预加载的裸数据与延迟加载的句柄。新建的资源存储用于隔离不同工程，而显式共享的存储支持在匹配的数据源与计数之间安全复用。`RenderOptions.cache` 是一个可克隆的 `RenderCache`，供多个会话与导出工作线程共享。覆盖率、段落排版、文字字形、渐变 LUT、数学公式以及显式的创作期计算分区各自拥有独立的条目与字节限制，并执行各自的 LRU 逐出策略。`SceneCache` 依然是专属的创作期计算存储，通过 Scene 分区共享。Studio 为每个注册的场景独立 fork 该分区，同时保留其余共享分区。这些缓存与解码后的资产及单帧光栅化总量相互独立。外部强引用的 `Arc` 与底层内存分配器开销不受上述逻辑配额的硬性约束。

## 执行边界与错误模型

无头（Headless）场景求值是完全同步的，会直接向上冒泡用户回调中的 panic。Studio 工作线程负责捕获 panic 并保留强类型错误信息，直至安全呈现。预览请求支持合并；导出任务严格渲染每一个选定帧。多个 GPU 上下文共享同一个底层设备，并对带作用域的提交进行序列化。显卡设备丢失会导致当前帧失败中止，而绝不悄然退化为 CPU 后端。

`RequestDiagnostics` 可通过 `Handle::get_with` 显式共享给后台资源加载线程。`DrawContext` 持有共享的不可变 `Evaluation`（涵盖时间、质量、资源、字体、诊断器及 RenderCache），加上设备缩放与验证限制；`SceneContext` 共享一份 `Arc<Evaluation>` 并仅保留局部/全局时间、片段时长、帧率、尺寸及其 `SceneCache`；`SceneContext::measure` 将该 evaluation 与局部时间交付给一个 `DrawContext`。`cache_layout` 始终使用该单一缓存中的 `CachePartition::Paragraph` 分区；不存在所谓的 FrameAssets 全局桥接。由请求派生的 Assets 视图及其句柄会继承诊断接收器；旧句柄在绘制边界处需要调用 `get_with`。`RenderContext` 携带资产与结构化限制；`RenderOptions.gpu_context` 允许注入共享的 GPU 设备。Studio 可以直接向同一设备的预览纹理提交绘制指令，无需全帧 CPU 回读。输出纹理必须通过设备与目标格式校验。

系统字体探测仅索引元数据；仅有被实际选中的字体源才会延迟加载。显式指定的字体文件/字节在注册时即被冻结。`ExportReport.fonts` 记录实际观测到的字体名称与源字节哈希值（包含缓存的字形绘制）。这些非密码学哈希用于源审计，而非用于跨工具链的像素光栅化一致性证明。

视频编码仅在文件最终完全写入成功且通过取消检查后，才将同目录下的临时输出文件原子提交覆盖。时间轴上的外部音频文件单独解析为感知时间区间的准备混音流；原生编码器在其专属线程上接收该混音。

## 包与特性管理

工作区根目录为虚包清单（Virtual Manifest）。其默认成员为 [crates/motion](../../../crates/motion/Cargo.toml)：package 为 `motion-rs`，库名为 `motion`。根目录下的示例配置了显式目标路径。未发布的基准测试包拥有自己的目标、测试固件与数据溯源构建脚本，独立于 facade 的构建脚本。

| 包名称 | 职责范围 | 引擎内部依赖 |
| --- | --- | --- |
| motion-core | 时间、几何数学、动画系统、场景/布局/时间轴、绘制指令、资源、音频事件；为基础值类型提供可选的 `serde`/`schema` | 无 |
| motion-render | 覆盖率计算、CPU/GPU 合成、滤镜、渲染会话、输出缓冲区、胶片 Look 风格 | core |
| motion-content | 图像、文本、SVG、矢量、粒子，可选的墨迹/图表/字幕；与特性无关的纯内容策略，提供可选的 `serde`/`schema` | core |
| motion-formula | Typst 文档/数学公式编译与公式求值 | core, content |
| motion-encode | PCM 准备、音频文件流式混合与事务性视频编码 | core, render；可选 FFmpeg |
| motion-scene | 版本化 JSON 文档、Schema 定义及其到时间轴的编译 | core, render, content；可选内容准备特性 |
| motion-studio | 桌面工作台、视口预览、播放控制与多任务导出编排 | core, render, encode |
| motion-rs | 引擎 facade 统一入口、prelude 预导入、Project/CLI 与场景元数据 | 经 Feature 条件选择的上述子包 |
| motion-ffmpeg | 原生 FFmpeg 绑定及线程隔离的编解码器/混音器底层资源 | 无 |
| motion-cli | 面向声明式 JSON 文档的独立 `motion` CLI 命令行程序 | facade |
| motion-bench | 引擎/短片基准测试套件与数据溯源 | facade, Gallery |
| motion-gallery | 创作完成的示范短片展厅 | facade |

仅有 facade 负责装配对外的公开创作 API。底层各子包仅以私有方式引用上游类型，严禁反向依赖 facade，也不允许把公开的 prelude 用作内部导入清单。扩展协议严格保留在定义它们的 crate 边界处。Core 仅定义不可变的图像/实例契约与冻结的字体快照，不引入具体的媒体引擎或硬件设备。kurbo 是几何运算依赖而非替代渲染器；Typst 内部细节完全封装在 formula 包中。

facade 默认启用特性：`gpu,image,svg,text,particles,ffmpeg,studio,ink,diagram,captions,declarative`。编译时包含某项特性并不代表运行时会隐式强制启用 GPU、探测字体、开启桌面窗口或播放音频。Gallery 禁用了 facade 的默认特性，仅转发其自身选定的能力。

| 特性依赖关系 | 边界与约束 |
| --- | --- |
| vector | 共享矢量内容与 Morph 变形，不包含图片解码、字体或粒子 |
| image | 包含 vector；启用静态图像解码 |
| svg | 需要显式的 image/text 准备设施以支持完整的 SVG 特性 |
| particles | 依赖 image，不依赖 text；支持独立开启 |
| diagram / captions | 依赖 text；作为带命名空间的内容工具对外暴露 |
| ink | 依赖 image；通过 vector::ink 命名空间暴露 |
| typst / formula | 可选的数学排版编译器 / 额外的数学公式创作与变形 |
| audio / ffmpeg | 无头音频混合 / 原生系统编解码器与视频音频封装 |
| studio | 桌面交互工作台；启用 FFmpeg，但不自动强制场景使用 GPU |
| serde / schema | core 与 content：为纯数据类型和策略提供 serde 与 JSON Schema 支持，由 motion-scene 开启而无需开启重量级内容引擎 |
| declarative | 声明式 JSON 文档；文本、图片和 SVG 元素按对应内容特性门控 |

facade 中的 authoring、extension、rendering 与 eager 命名空间各自承担不同职责。日常创作导入请参考[教程指南](../tutorial/README.md)，环境隔离验证参见[开发指南](../development.md)，基准测试命令参见[性能基准](../benchmarks.md)。

## 源码目录结构

库的入口模块负责声明子模块并导出公开类型。Formula 和 Studio 的核心实现直接位于 `src/` 根目录下，而非再嵌套一层业务领域目录。纯测试套件与被测实现就近放置；大型实现文件的拆分与测试文件体积分别独立控制。

- Core: authoring 场景/节点/修饰器/上下文/摄像机/骨骼、asset 资产、color/math 颜色与数学、drawing 绘制与 shape 形状。路径 Morph 归属 path2d/morph。
- Render: prepared 拥有不可变记录；raster 负责共享准备流程；cpu 与 gpu 负责消费。合成时钟、会话与具体执行相互解耦。
- Formula: [编译器](../../../crates/motion-formula/src/compiler.rs)、[帧转换器](../../../crates/motion-formula/src/frame.rs)、[操作逻辑](../../../crates/motion-formula/src/operations.rs) 与 [Typst 文档编译器](../../../crates/motion-formula/src/typst_asset.rs)。
- Encode: [导出编排](../../../crates/motion-encode/src/export.rs) 与 [FFmpeg 编码器](../../../crates/motion-encode/src/encoder/ffmpeg.rs)；原生事务适配器为 crate 私有。
- Studio: app、panels、viewer、timeline、playback、session、preview worker 与 export 模块；音频预览/配置/文件准备与 core 保持解耦。

性能优化与特性拓展的优先级排列在[开发规划](../plan.md)中。基准测试规范将诊断性引擎用例与实际的 Gallery 短片严格区分开来。它们生成的数据汇入唯一的[性能基准报告](../benchmarks.md#当前实测数据)，与历史基线及上一阶段进行客观比对。
