# 附录：Gallery 展厅指南

Gallery 展厅汇集了完整的电影级短片作品，同时作为动画、渲染、多媒体内容、粒子与音频特性的参考范例库。建议在深入各章节具体实现细节之前，先在 Studio 中仔细观察它们的宏观结构、视觉关系与动画节奏。

## 运行入口与字体配置

```sh
cargo run -p motion-gallery --bin motion-gallery -- list
cargo run -p motion-gallery --bin motion-gallery -- still type-in-motion --time 4 --font tests/fixtures/fonts/NotoSans-Regular.ttf --output type.png
cargo run --release -p motion-gallery --features studio,gpu --bin motion-gallery -- studio --demo type-in-motion --font tests/fixtures/fonts/NotoSans-Regular.ttf
```

`list` 子命令不会初始化任何字体、数学公式、音频硬件或 GPU。其余命令接受显式的 `--font` 参数。若未指定，Gallery 会在应用启动时一次性探测系统字体，绝不会在核心帧回调中隐式扫描磁盘。

默认提供 18 部短片，开启 `formula` 特性后提供 19 部；未启用的可选条目不会出现在目录中。短片默认片长 8 秒，设计画布尺寸 960x540，严格 30 fps。`--params` 支持配置 1–60 秒的 `seconds`、固定随机种子以及 4–128 的 `count` 数量。

能力图谱（Atlases），包括粒子特性图谱与墨迹特性图谱，排在短片目录与 CLI 列表的最前面。Studio 在未显式指定 `--demo` 时默认打开动画图谱（`animation-atlas`）。下表按照短片目录的标准顺序排列。

## 按学习目标选型导引

| 短片 ID / 标题 | 核心学习章节 | 重点观察的特性与细节 |
| --- | --- | --- |
| `animation-atlas` / 动画图谱 | [03 动画系统](03-animation.md) | 包含 43 组独立实验：采样曲线、实时标记、缓动、弹簧阻尼、振荡器、路径跟随与时间组合子 |
| `brush-atlas` / 笔触图谱 | [06 内容与资源](06-content.md), [09 渲染与合成](09-rendering.md) | 10 页，每页对应一种官方画笔预设、共享几何路径、预设压力与笔尖参数、水墨材质、正片叠底合成与 Bloom 泛光 |
| `particle-atlas` / 粒子图谱 | [07 粒子系统](07-particles.md) | 每页对应一种粒子预设，包含定步模拟、拖尾、程序化精灵图、碰撞飞溅、烟花与带颜色的矢量汇聚 |
| `particle-features` / 粒子特性图谱 | [07 粒子系统](07-particles.md) | 47 组深度实验，覆盖发射器、运动轨迹、外力场、群体算子、物理碰撞、事件、样式、材质、时间控制与多线程一致性 |
| `filter-atlas` / 滤镜图谱 | [09 渲染与合成](09-rendering.md) | 17 组滤镜、镜头光学效果、HDR 高光与滤镜串联顺序对比 |
| `compositing-atlas` / 合成图谱 | [05 绘制与图层](05-drawing.md), [09 渲染与合成](09-rendering.md) | 26 组混合模式、内容遮罩、几何裁剪、组不透明度、兄弟层级与图层隔离对比 |
| `vector-atlas` / 矢量图谱 | [05 绘制与图层](05-drawing.md), [06 内容与资源](06-content.md) | 13 组路径修剪、虚线描边、绕数规则、高级渐变、SVG 导入、Morph 变形与手写显现 |
| `ink-features` / 墨迹特性图谱 | [06 内容与资源](06-content.md), [09 渲染与合成](09-rendering.md) | 21 组压力响应、笔尖方位、宣纸材质、采样输入、提笔停顿、橡皮擦除与表面重新打光 |
| `noise-atlas` / 噪声图谱 | [02 核心基础](02-foundations.md) | 12 组标量与向量场，包括分形 fBm、域扭曲、解析法线梯度与旋度向量 |
| `audio-atlas` / 音频图谱 | [10 音频处理](10-audio.md) | 21 组带声音的实验：波形合成、音量包络、滤波 DSP、空间混音与母带限幅 |
| `typography-atlas` / 字体排版图谱 | [06 内容与资源](06-content.md) | 18 组中英文排版实验：富文本 Span、字重、字号、段落折行、字间距、字形轮廓、渐变、逐字变换与手写 |
| `caption-atlas` / 字幕图谱 | [06 内容与资源](06-content.md) | 12 组中英文字幕实验：安全矩形排版、时间轴时序、SRT 导入导出与共享时间线索标记 |
| `particle-garden` / 粒子花园 | [07 粒子系统](07-particles.md) | 子发射器级联、物理碰撞体、Boids 鸟群算法、流场引导与画刷感知发射器 |
| `data-story` / 海港渡轮 (The Harbor Line) | [03 动画系统](03-animation.md), [06 内容与资源](06-content.md) | 统计图表比例尺、折线图、关键帧数字跳动、`Row`/`Column` 自适应卡片与字幕轨道 |
| `logo-sting` / 品牌演绎 (Motion) | [16 声明式 JSON 文档](16-declarative.md), [06 内容与资源](06-content.md) | Motion M 字母标与全小写字标，将 JSON 声明式文档与矢量 Morph、修剪路径及弹簧物理动画无缝复合 |
| `type-in-motion` / 空间游弋 (Room to Move) | [03 动画系统](03-animation.md), [06 内容与资源](06-content.md) | 诗意排版、字素簇弹簧动画、段落首字下沉、版面转场 |
| `ink-journey` / 漫漫归途 (The Long Way Home) | [06 内容与资源](06-content.md), [09 渲染与合成](09-rendering.md) | 连续毛笔笔触、宣纸浸染材质、摄像机缓动漂移、老胶片噪点与漏光伤痕 |
| `signal-garden` / 信号花园 (Signal Garden) | [07 粒子系统](07-particles.md), [10 音频处理](10-audio.md) | 5 组 FFT 频段实时驱动径向频谱、Plexus 连线星系与 HUD 仪表盘 |
| `harmonics` / 和声共振 (Harmonics) | [08 Typst 与数学公式](08-formula-typst.md), [03 动画系统](03-animation.md) | 数学公式语义匹配插值与倍频波形几何协同演化 |

## 海港渡轮 (The Harbor Line)

[源码实现](../../../gallery/src/demos/data_story.rs) 讲述了一个关于虚构海港渡轮的数据故事：基于比例尺映射的柱状图逐年增长，折线图记录通勤时间平稳下降，关键帧计数器在统计卡片行中飞速滚动，底层配合时间对齐的字幕进行叙事旁白。
重点学习图表比例尺如何将业务数据映射为几何图形、`Row`/`Column` 如何自动测量卡片排版，以及单条字幕轨道如何与图表共享时间轴时钟。

## 品牌演绎 (Motion)

[源码实现](../../../gallery/src/demos/logo_sting.rs) 是 Motion 的官方品牌演示短片。一个 `motion::declarative` [JSON 文档](../../../gallery/assets/solstice-ident.json)负责绘制网格背景、光芒爆发、全小写 `motion` 字标与宣传语；Rust 代码则利用弹簧将三个基础图元弹射入场，将其 Morph 变形为经典的三笔画 M [SVG 图标](../../../gallery/assets/solstice-mark.svg)，并在其周围环绕一道被动态修剪的轨道光弧。可直观体会声明式 JSON 文档与代码原生动画的最佳融合边界。

## 动画图谱 (Animation Atlas)

[源码实现](../../../gallery/src/demos/animation_atlas.rs) 在统一的图谱布局中展示了 43 组独立实验：左侧为标题与原理解释，右侧为带裁剪的动态实物画面。全部 24 种缓动曲线拥有独立页面，随后展示三种弹簧阻尼状态、四种振荡器波形、带种子噪声、路径跟随器、阶梯色调/变换，以及 9 组时间组合子范例。

曲线与数值直接对真实的动画对象进行采样。它们各自独立的 3.2 秒局部时钟确保无论整部短片总片长如何调整，超调、有符号信号、延迟与关键帧保持均能完整重现。运行 `studio --demo animation-atlas --params '{"seconds":60}'`，随后暂停或手动 Seek 即可细致比对每一页。

## 笔触图谱 (Brush Atlas)

[源码实现](../../../gallery/src/demos/brush_atlas.rs) 通过 10 个独立页面系统比对全部 10 种 `BrushPreset` 笔触预设，每页包含原理解释与独立裁剪视口。运行 `studio --demo brush-atlas --params '{"seconds":40}'`，每页停留 4 秒。共享的几何路径与标称线宽清晰呈现出内置压力响应、笔尖方位、纸张颗粒、半透明叠色与 Glow 发光的细腻差异。浅色宣纸突出水墨与半透明预设；Neon 霓虹画笔则搭配深色底与局部 Bloom。

每个色块保留其完整的材质准备产物，并通过动态几何裁剪驱动笔迹显现，而非在每个显现步长重新烘焙材质。本示例显式准入单笔画高达 6400 万次材质距离计算；引擎底层默认上限为 3200 万次。

## 粒子图谱 (Particle Atlas)

[源码实现](../../../gallery/src/demos/particle_atlas.rs) 按照 `Preset::ALL` 的标准顺序完整呈现 13 种官方粒子预设。每页左侧展示预设名称与简要描述，右侧为带裁剪的物理模拟视口。运行 `studio --demo particle-atlas --params '{"seconds":52}'`，每页展示 4 秒。

物理模拟在各帧之间完备保留显式的随机种子与已准备状态。页面进度映射到有界的局部采样时间窗口，确保短命的火花（Sparks）与烟花（Fireworks）在短片总时长变化时依然清晰可见。Logo 预设共享现有的彩色矢量源；Confetti 采用浅色背景，而 Smoke 与 Fire 则采用更紧凑的局部特写视口。HDR 粒子在视口内部叠加 Bloom 泛光。无需外部图像文件或调大全局预算。

## 粒子特性图谱 (Particle Features)

[源码实现](../../../gallery/src/demos/particle_features.rs) 包含 47 组深度能力实验，而非预制配方。左侧为文字说明，右侧为裁剪视口。运行 `studio --demo particle-features --params '{"seconds":60,"count":96}'` 进行长片漫游，在 Studio 中暂停或跳转以自检特定页面。

发射器实验涵盖点源、圆盘、矩形、瞬时爆发、矢量路径、矢量填充表面、位图 Alpha/亮度加权以及字形轮廓源。解析弹道、汇聚、轨道与噪声漂移轨迹与基于定步长的外力场、Boids 鸟群、弹簧与碰撞实验清晰解耦。圆形、多边形、线段与 SDF 碰撞体全面演练了真实的物理碰撞反馈与次级派生事件。外观实验比对纯几何图形、历史拖尾与飘带、Plexus 连线、散景虚化、混合模式、程序化精灵图材质、帧动画选择、烟雾演化、速度拉伸、翻转、生命周期曲线、颜色场与色温。

尾部页面展示了预热、无缝循环、慢速与倒放时间重映射，以及单线程与四线程完全一致的模拟输出。显式随机种子与局部源时间使得 Seek 定位重放绝对独立于观看顺序。各模拟显式允许 4 亿工作量单位，引擎默认底层配额保持不变。

## 渲染、内容与音频图谱

上述各大能力图谱共享统一的排版布局：清晰的左侧无衬线标题说明搭配右侧带裁剪的动态实测画面。运行 60 秒全局巡览，只需替换 `--demo` 后的 ID 即可无缝切换不同主题：

```sh
cargo run --release -p motion-gallery --features studio,gpu --bin motion-gallery -- studio --demo filter-atlas --params '{"seconds":60}'
```

[滤镜图谱](../../../gallery/src/demos/filter_atlas.rs) 在两侧使用完全相同的画面源（包含真实的 HDR 超白高光），并在最后一页直观对比不同的滤镜串联顺序。[合成图谱](../../../gallery/src/demos/compositing_atlas.rs) 使用共享的重叠几何形状展示全部 17 种公开混合模式，其后半部分深入剖析了遮罩通道与反转、裁剪模式、子节点 vs 组不透明度、同级兄弟层级以及隔离图层滤镜。

[矢量图谱](../../../gallery/src/demos/vector_atlas.rs) 实际演练了路径修剪（Trim）、虚线（Dash）与描边外轮廓几何计算、三种渐变类型、边界外延模式与色彩插值空间。最后几页展示原生 SVG 矢量的 Morph 变形与动态书写。[墨迹特性图谱](../../../gallery/src/demos/ink_features.rs) 将画笔核心底层能力从预设配方中独立提炼出来。录制的采样点保留压力、倾斜角、时间戳与停顿驻留；定时书写拥有真实的提笔间歇。颜料混色对比了线性光叠加与 6 频段手写反射率近似模型。

[噪声图谱](../../../gallery/src/demos/noise_atlas.rs) 基于显式种子与采样坐标生成标量纹理。分形页面显式采用 5 个倍频。法线梯度与旋度向量箭头全部基于解析导数直接计算，而非有限差分近似。

[音频图谱](../../../gallery/src/demos/audio_atlas.rs) 包含真实的 48 kHz 立体声音频，绝非纯画面的虚假波形。干声单声道与处理后的左/右声道读取真实 PCM，绘制全页峰值包络与共享的可视化电平。处理后的混音在 Studio 与视频导出中真实可听；干声音频作为视觉对比基准展示，不作为并发混音输出。短促的瞬态冲击与效果器尾音在特定时刻可能是静音的，即便其全页静态包络可见。无需外部音频文件即可体验纯波形合成。

## 字体排版与字幕图谱

[字体排版图谱](../../../gallery/src/demos/typography_atlas.rs) 聚焦中英文双语排版。18 个页面系统比对富文本切片（Span）、字重、字号、自动换行折行、行间距以及全部四种段落对齐方式，随后展示字形矢量轮廓、字符包围盒渐变、字素簇/单词/折行可见性、逐字独立空间变换、矢量手写动画与稳定的局部排版缓存。显现字符数严格来源于塑形后的段落切片，而非原始字节偏移。运行 `studio --demo typography-atlas --params '{"seconds":60}'` 开启长篇自检。

[字幕图谱](../../../gallery/src/demos/caption_atlas.rs) 包含 12 组基于真实 `CaptionTrack`、`CaptionStyle` 与 `CueSheet` 的实验。在字幕旁边可直观观察防遮挡安全矩形与局部时间刻度：自动折行、内边距与并发多条字幕堆叠在采样缓存矢量前完成测量。精准的左闭右开时间边界杜绝了在剪辑拼接处产生多余的闪烁重影。SRT 往返序列化采用毫秒级时间戳；共享标记点驱动同频视觉动画，最后一页对比了 4 秒原版字幕表与 6 秒重定时版本在相同局部时间下的表现。运行 `studio --demo caption-atlas --params '{"seconds":48}'`，每页展示 4 秒。

两组排版图谱均使用应用启动时一次性加载的操作系统字体库，不绑定专有字体文件。中文字符需要系统安装有受支持的中文字体；文本排版引擎会自动调用字面回退逻辑。字体探测绝不在帧求值回调内部发生。像素光栅化结果取决于当前计算机安装的具体字体，而同一环境下的时间轴 Seek 定位与重放是绝对精确的。

## 空间游弋 (Room to Move)

[源码实现](../../../gallery/src/demos/type_in_motion.rs) 采用短片 ID `type-in-motion`。
封面设计采用基于物理弹簧平息的字素簇动效；中间跨页采用排版首字下沉（Drop cap）、重点引言以及首端对齐的双栏段落；收尾页采用全出血震撼排版。遮罩与定向划像维持着出版物般的优雅韵律。
仔细体会[文本与排版](06-content.md)如何决定空间布局、动画如何直接以字素簇而非 UTF-8 字节下标为单位驱动，以及静态版面如何与动态文字相得益彰。

## 漫漫归途 (The Long Way Home)

[源码实现](../../../gallery/src/demos/ink_journey.rs) 在泛黄的复古做旧纸张上，融汇了动态压感毛笔笔触、打字机逐字字幕、印章/日期标记、空气微尘、镜头划痕、边缘漏光与逐帧胶片颗粒。
体会连续笔触的动态显现、压力与墨色时间线，以及摄像机的微妙缓动漂移。完全静止的笔触能够直接复用已烘焙的材质产物；而正在绘制的活跃笔触则按时间进度动态展开。

## 信号花园 (Signal Garden)

[源码实现](../../../gallery/src/demos/signal_garden.rs) 在代码中纯合成底鼓、踩镲、贝斯与主旋律音色，通过带侧链闪避、延迟/混响、立体声扩展与母带限幅器的音频总线进行混音，随后回读混音信号。FFT 频段能量实时驱动径向频谱与 Plexus 星系连线，测得的闪避增益让人声与背景随着底鼓有力律动，总线电平表实时驱动 HUD 仪表盘；雷达扫描线、快门时间采样与虚化光斑营造出深邃的空间深度。

## 和声共振 (Harmonics)

开启 Gallery 的 `formula` 特性。[源码实现](../../../gallery/src/demos/harmonics.rs) 中展示了 Typst 数学公式源码、子式提取以及语义匹配插值，将其与频率翻倍的物理波形振动紧密联动。
公式是原生的 Typst 矢量图形，绝非简单的公式截图。几何形状的拓扑配对不代表引擎理解任意高等数学等价性。安全信任边界与[第 08 章](08-formula-typst.md)完全一致。

## 客观解读基准性能

Gallery 遵循未改动的引擎底层创作配额以及 BrushStroke 的 3200 万默认材质配额，其中在 Brush Atlas 中显式放宽至单笔画 6400 万，在 Ink Features 中放宽至 1280 万。Particle Features 显式将单项模拟的工作量上限从 1 亿提升至 4 亿；粒子数量与显存字节上限保持不变。终版墨迹按实际索引计算空间距离场；画面复杂度与连续笔画未做任何削足适履的简化。详见[默认预算契约](../../../gallery/README.md#default-budgets)。这些上限不代表物理实时播放保证，也不是系统物理 RSS 内存上限。

这些短片直接充当[短片性能基准](../benchmarks.md)的真实工作负载。性能优化必须在严格保证完整图层合成、全分辨率自检、精确重放与 CPU/GPU 视觉一致性的前提下推进。静默简化艺术表现绝不叫作性能优化。

## 常见踩坑点

- 误将画面展示的宣传标题当作命令行注册的短片实际 ID。
- 在未固定字体的两台不同机器之间盲目比对最终光栅化像素。
- 误把可选的 Harmonics 公式短片当作无需特性即可运行的默认目录项。
- 为了迎合预算而随意删减伴音、安全提示或截断连续笔画。
- 把 Gallery 为了演示极端效果而显式配置的局部配额当作库的默认全局参数。

## 进一步阅读

返回[教程目录](README.md)。查阅 [Gallery README](../../../gallery/README.md) 了解项目入口、参数选项与完整片单。
在开展优化工作前请通读[性能与可靠性](13-performance-and-reliability.md)，在调整核心接口前请研读[架构与扩展](14-architecture-and-extending.md)。
