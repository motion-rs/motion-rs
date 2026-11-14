# 15 电影级视觉输出

所有电影级画质处理功能均为显式可选开启项。`RenderOptions::default()` 保持原始直出渲染路径。导出的浮点 RGBA16F 画面始终保持场景线性空间；显示变换仅作用于 SDR RGBA8 与 YUV 输出。Alpha 透明通道保持不变。

```rust,no_run
use motion::prelude::*;
let display = DisplayTransform {
    tone_map: ToneMap::AgX,
    reference_white: Some(1.0),
    grain: Some(FilmGrain {
        seed: 42, amount: 0.012, size: 1.5,
        luma_response: 1.0, chroma: 0.1,
    }),
    ..DisplayTransform::default()
};
let options = RenderOptions::default().display(display);
```

管线处理顺序严格为：曝光度（Exposure）→ CDL 调色 → 风格 Look LUT → 色调映射曲线（Tone Curve）→ 饱和度（Saturation）→ 显示 LUT → 输出传输特性（OETF）→ 胶片颗粒（Film Grain）→ Bayer 抖动 → 色阶量化。参考白归一化使用 `curve(rgb*k)/curve(k)`。Khronos PBR Neutral 标准曲线包含微小的暗部偏移：它在 0.76 以下并不完全等价于直通，在 1.0 处也不绝对保持 SDR 纯白。ACES 采用 Hill 的 RRT/ODT 拟合曲线。AgX 在其 gamma-2.2 多项式后、在输出 OETF 之前执行线性化。Hable 曲线支持显式指定归一化白点。

## 空间光学滤镜效果

`Filter::Bloom` 采用柔和高光提取、归一化的 Octave 权重与有界高斯模糊核，在 2 倍降采样金字塔上加速计算。`Filter::halation`（光晕溢出）与 `Filter::anamorphic`（变形镜头光斑）为现有基础图元的纯透明组合。`FilterOperation::Transform` 以透明边界对像素中心执行逆向映射。`BlurXY` 具备独立的水平与垂直 sigma；局部 sigma 使用父级的最大奇异缩放比，即使父级节点发生旋转或倾斜，其模糊轴向始终与设备像素网格对齐。

`Filter::Optics` 支持径向镜头畸变与色差色散、物理四次方余弦暗角（Cos-fourth Vignette）、8 至 64 个随半径缩放采样点的圆形/多边形散景虚化（Bokeh）、有界径向模糊、Sobel/Lambert/Blinn-Phong 表面重打光、带环形加权的镜像镜头光斑鬼影（Lens Flare Ghosts），以及 `Optics::Anamorphic` 水平变形拉丝光斑。散景虚化属于光圈光学近似，而非深度缓冲区渲染器。镜头/径向/仿射/光斑重采样要求全帧 CPU 图像支撑；模糊与重打光保留有界的分带光晕。记录画布范围之外的外部光源无法贡献进画面。这些操作全额计入光栅工作量与存储预算，可能在渲染前被准入系统拦截。

`Optics::Anamorphic { threshold, length, stretch, tint, intensity }` 提取高于 `threshold` 的强高光，沿水平轴以 `length`、沿垂直轴以 `length / stretch`（`stretch` 在 1..=64 范围内，对齐设备坐标轴）进行非对称模糊，赋予色调并以 Plus 模式叠加。它与 Glow 风格共享 CPU/GPU 管线，轴向包围盒外扩 `ceil(3 sigma)` 并计入预算。

`DepthOfField`（景深）按从远到近的顺序捕获显式的 `DepthPlane` 深度平面。物体距离与镜头焦距共享场景设计单位。薄透镜弥散圆（CoC）在对焦点处精确为零，上限受制于 `max_radius`；可选的针孔透视同时缩放几何体与摄像机平移。每个失焦平面被包装进对应半径的 `Optics::Bokeh` 中，因此景深完全由常规的 CPU/GPU 光学管线统一渲染；平面坐标以摄像机为中心。`NodeModifiers::layered_shadow` 在单个源图背后合成三层渐进的 Alpha 剪影轮廓。`Elevation::shadow(height)` 搭配 `NodeModifiers::elevated` 自动构建接触阴影、主投影与环境微光阴影，其模糊度和偏移量随高度增加而扩散，不透明度平滑衰减；高度为 0 时不增加任何额外开销。`gate_weave`（放映机胶片抖晃）接收显式的随机种子与帧索引，不维护任何隐式时钟或可变状态。

## 调色分级与 3D LUT

`Cdl::temperature_tint` 组合基础通道增益，而非物理光谱白平衡。`ColorWheels`（三路色轮）在 0.18 中性灰周围平滑调整暗部（Lift）、中间调（Gamma）与高光（Gain）；`bake(size, domain_max, cdl)` 生成不可变的原始 LUT，可同时用于 `DisplayTransform::look_lut` 与 `Filter::Lut`。

`motion_core::Lut3d::from_cube` 接收 2..=64 尺寸的有界 3D Cube 文件，红通道快速递增存储与显式定义域，前面可挂载 2..=65536 采样的 1D Shaper 预塑形曲线。`Lut1d::from_cube` 读取独立的 1D 文件。文件加载通过 `motion_content` 中的 `load_lut` 与 `load_shaper` 显式执行（同样由 facade 的 `vector` 特性导出）。不支持的指令、重复头部、非有限数值与采样点不匹配会被直接拒绝；文件大小上限为 8 MiB。

每个 Shaper 采样行包含独立的 R/G/B 数值，执行线性插值。管线处理顺序严格为：输入色彩编码 → Shaper 映射 → 四面体插值 3D Cube → 输出色彩解码。Shaper 与 Cube 均不是隐式的输出传输函数。超出定义域的输入会被截断，但 Shaper 的输出可以是负数或 HDR 宽容度数值。在复合文件中，DOMAIN_MIN/MAX 描述 Shaper 的输入范围；Cube 默认为 0..1。`LUT_1D_INPUT_RANGE` 与 `LUT_3D_INPUT_RANGE` 提供显式的标量区间。若同一阶段同时出现 DOMAIN 与 INPUT_RANGE 冲突，视为歧义直接报错。在代码中构造时可独立指定各阶段单通道的定义域：

```rust
use motion::prelude::*;
let shaper = Lut1d::new(
    [0.0; 3], [16.0; 3],
    vec![[0.0; 3], [0.8; 3], [1.0; 3]],
)?;
let look = ColorWheels::default().bake(3, 1.0, Cdl::default())?
    .with_shaper(shaper);
assert!((look.sample([8.0; 3])[0] - 0.8).abs() < 1e-6);
# Ok::<(), motion::ValidationError>(())
```

CPU 与 GPU 采用相同的插值算法与 Linear/sRGB/Log 编码契约。GPU 端 LUT 数据保存在 Storage Buffer 中；子树滤镜与全局 Look/Display LUT 均完整支持 Shaper 预塑形曲线。

无需第三方 LUT 授权即可快速生成专有调色预设：

```sh
node tools/generate-lut.cjs teal-orange look.cube 33
```

## 程序化大气环境 (Atmosphere)

`motion::atmosphere::Atmosphere`（需要 `vector` 特性）提供可配置的 `fog(seed)`（雾气）、`dust(seed)`（悬浮微尘）与 `light_leak(seed)`（镜头漏光）配方。它们通过标准的图像管线生成不可变的线性 RGBA16F 图像资产，完全不需要外部图像解码库。色调、不透明度、HDR 强度、像素缩放比、漂移速度与分形参数均支持精确配置。雾气具备垂直密度渐变；微尘具备确定性的稀疏柔和颗粒；漏光具备归一化的椭圆发光包络线。

```rust
use motion::prelude::*;
use motion::atmosphere::Atmosphere;
use motion::render::RasterLimits;
let mut fog = Atmosphere::fog(42);
fog.scale = 80.0;
let asset = fog.render(PixelSize::new(64, 48), 1.seconds().after_origin(), RasterLimits::sandbox())?;
let overlay = Node::from(Image::from_asset(std::sync::Arc::new(asset)))
    .blend_mode(BlendMode::Screen);
# let _ = overlay;
# Ok::<(), motion::Error>(())
```

`Atmosphere::overlay(size, time, limits)` 返回相同尺寸的双线性过滤节点，并自动配置推荐的混合模式（雾气使用 Screen 滤色，微尘与漏光使用 Plus 线性减淡）。雾气遮挡推荐使用 SourceOver，微光高光叠加使用 Screen/Plus。在不同时刻 `Time` 求值会驱动连续的二维流体平流推进；这绝非物理粒子积分，也不是三维体渲染散射计算。结合不同深度的平面雾气配方可创作出层次分明的 2.5D 大气透视。`render_region` 支持局部瓦片渲染并保持全局坐标与全画布包络一致。

`motion::noise` 提供 `hash32/hash2/hash3`、`value2/value2_d`、`perlin2/perlin2_d`、`curl2`、`perlin3`、`simplex2`、`worley2`、`domain_warp`（域扭曲）、`turbulence2`、`ridged2` 以及幅度归一化的 `fbm`（分形布朗运动）。空间坐标与输出计算统一采用 f64。`Fractal` 准入 1..=8 个倍频（Octaves），粗糙度（Lacunarity）1..=8，增益 0..=1。在大气参数中，所有倍频坐标必须严格落在 32 位有符号整数晶格之内。GPU 渲染生成的画面，底层着色器采用共享的 `noise.wgsl`，拥有完全匹配的整数哈希、空间算法与 f32 插值。

```rust
use motion::noise::{DomainWarp, Fractal, domain_warp, fbm, simplex2, worley2};
let point = domain_warp([0.37, -1.23], DomainWarp { seed: 42, ..DomainWarp::default() })?;
let cloud = fbm(|point| simplex2(point, 19), point, Fractal::default())?;
let (nearest, second) = worley2(point, 19);
let cell_edge = second - nearest;
assert!(cloud.is_finite() && cell_edge >= 0.0);
# Ok::<(), motion::Error>(())
```

域扭曲（Domain Warp）输出偏移后的坐标，不自动改变内置大气配方，也不模拟物理焦散。显示层的 `FilmGrain` 胶片颗粒复用 `noise::film_grain`，其高斯单元与其它噪声函数共享底层晶格哈希。二维旋度噪声与演化烟雾材质已在[粒子系统教程](07-particles.md)中详细讲解。

## 快门时间采样与母带交付

`ShutterSampling` 增加显式带种子的分层抖动（Stratified Jitter）、盒式/三角/余弦曝光权重以及 8..=64 的自适应采样点选择。它既可以通过 `.capture(blur, context, output_size, build)` 针对选定子树进行局部快门采样，也可以直接安装在 `RenderOptions::shutter_sampling` 上实现整帧快门采样。任何整帧快门采样策略均严禁包含嵌套的子树快门采样。自适应采样通过比对端点在设备空间的几何位移判定；对于回到相同端点的周期性往返运动会自动回退至 64 次采样。对于这类周期性剧烈运动，建议显式指定固定采样点数。

```rust,no_run
use motion::prelude::*;
let look = Look::film(42);
let options = ExportOptions::new("film.mp4")
    .codec(Codec::h265(18).with_preset(EncoderPreset::Slow).ten_bit())
    .render(|options| look.render_options(options));
```

通过 `look.scene(context, output, build)` 构建每一帧：它首先执行快门采样积分，随后对累积完成的单帧整体应用空间滤镜（而非在每个快门采样点上重复运行滤镜）。`render_options` 仅注入显示变换配置。电影节奏建议将时间轴帧率设置为 24 fps。`Look::clean()` 为纯净直出显示模式，`cinematic()` 注入克制的泛光/镜头暗角/快门采样，`film(seed)` 进一步添加胶片颗粒与胶片抖晃。配方本身属于完全可编辑的纯数据结构。

`Look` 将 `look.effects`（`SceneEffects`：滤镜、快门、胶片抖晃，作用于场景）与 `look.display`（`DisplayTransform`：输出色彩分级）成对绑定。`look.into_parts()` 支持分别解构提取这两部分。`DisplayTransform.look` 在色调映射之前对已合成的场景线性帧执行色彩分级，而 `effects.filters` 内部的 `Filter::Cdl` 则是在场景合成前作用于当前图层节点的滤镜。

Main10 10-bit 高位深输出严格限定于 H.265。原生编码路径根据 FFmpeg 字节跨度写入小端序 16 位平面字节流。当视频宽度为 8 的倍数且高度为偶数时，GPU 导出流水线直接从最终的线性浮点纹理将 10-bit Y/U/V 打包为显存数据（GPU 导出与 CPU 导出的色阶误差在 1 个码值以内）。每个像素分量占据小端序 u16 的低 10 位，而非 P010 的高位对齐方式。回读数据每像素占 3 字节；FFmpeg 依然在 CPU 侧接收平面字节流，因此这并非纯显存零拷贝编码。其他偶数分辨率在 GPU 线性回读后由 CPU 执行颜色格式转换。元数据固定标记为 BT.709 基色/转换矩阵与受限范围；sRGB 色彩传输标记为 IEC 61966-2-1。`OutputTransfer::Gamma24` 在 YUV 下编码为纯 2.4 幂次曲线并标记为 BT.709，符合主流 SDR 母带交付规范。`params` 参数串大小受限，仅在 `Codec::H265` 上可用。

运行内置的画质对比示例：
`cargo run -p motion-rs --all-features --example cinematic -- clean` 与
`cargo run -p motion-rs --all-features --example cinematic -- film`。
两者在相同确定性运动的 HDR 场景源上生成 Main10 视频。P6 画质客观测量数据详见[画质评测报告](../quality-report.md)。

向上述命令追加 `--still` 参数可在 `docs/assets/` 目录下生成单帧对比静帧（不覆盖已有文件）。归档的标准对比基准为：[纯净版](../../assets/cinematic-clean.png) 与 [胶片版](../../assets/cinematic-film.png)。
