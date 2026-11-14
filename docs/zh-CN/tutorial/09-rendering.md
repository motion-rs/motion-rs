# 09 渲染与合成

## 本章学习目标

掌握 CPU/GPU 双渲染后端的选型策略，学会跨帧复用准备结果与多级缓存，并清晰区分带透明通道的位图、显式内存像素帧以及外部 GPU 纹理。
CPU 基础示例无需开启任何额外特性。原生 GPU 硬件渲染与外部纹理支持需要开启 `gpu` 特性。

## 最小可运行示例：复用 RenderSession

```rust,no_run
use motion::prelude::*;
use motion::render::{
    FrameAt, FrameBuffer, FrameRequest, PixelFormat, RasterLimits, RenderSession, RenderTarget,
};

fn main() -> motion::Result {
    let size = PixelSize::new(320, 180);
    let options = RenderOptions::default().size(size);
    let mut session = RenderSession::new(Size::new(320.0, 180.0), options)?;
    let mut output = FrameBuffer::new(size, RasterLimits::default())?;
    session.render(
        FrameRequest {
            at: FrameAt::Time(250.milliseconds().after_origin()),
            frame_rate: FrameRate::NTSC,
            duration: 2.seconds(),
            background: None,
            target: RenderTarget::Memory(&mut output),
            format: PixelFormat::SrgbRgba8 { premultiplied: false },
            device_packing: false,
        },
        |context| Node::from(context.bounds().fill(Color::rgb(20, 150, 125))),
    )?;
    Ok(())
}
```

渲染产物以声明的像素格式原子写入内存目标缓冲区。后续的渲染请求可以直接复用同一个渲染会话（Session），避免为每个时刻点重复初始化底层硬件设备和全部缓存。

## 选择合适的 API 层级

单张静态图片生成请选用 `render_still`，任何涉及重复多帧渲染的场景请选用 `RenderSession`。其余层级面向整数索引时钟与底层协议扩展。经过校验的 `Composition` 统一通过 `Composition::for_timeline(timeline, options)` 构建。

| API 层级 | 最佳适用场景 |
| --- | --- |
| `render_still(timeline, time, options)` | 渲染单张带透明通道的高质量静态图片 |
| `RenderOptions` | 共享的渲染后端策略、资产库、画质等级、输出分辨率与预算配额配置 |
| `RenderSession` + `FrameRequest` | 针对显式时刻、目标表面与像素格式的高频重复请求 |
| `Composition` + `FrameRenderer` | 面向特定合成的整数索引单帧渲染 |
| `RenderPrepare` / `PreparedCpuFrame` | 控制指令准备与底层图元消费的高级扩展 |
| `GpuContext` | 导入或共享底层显卡原生设备、命令队列与指令提交生命周期 |

`FrameAt::Time` 绝不自动吸附到帧网格点；`FrameAt::Index` 则严格根据请求的有理数帧率进行精确计算。`FrameRenderer` 为特定合成的帧索引封装了相同的会话，因此两者的时钟系统必须保持清晰分离。

## 渲染后端策略选择

```rust,no_run
use motion::prelude::*;

fn main() {
    let cpu = RenderOptions::default().backend(BackendPreference::Cpu);
    let default = RenderOptions::default();
    let strict_gpu = RenderOptions::default().backend(BackendPreference::gpu());
    let ci_gpu = RenderOptions::default().backend(BackendPreference::Gpu {
        allow_software: true,
    });
    let _ = (cpu, default, strict_gpu, ci_gpu);
}
```

上述代码仅配置策略参数，绝不隐式初始化 GPU。实际发起 GPU 渲染需要开启 `gpu` 特性并具备受支持的本地显卡适配器。

- 默认策略：开启 `gpu` 特性时默认为 `Gpu`，未开启时为 `Cpu`。
- `Cpu`：显式选择参考级纯软件光栅化器，完全不初始化任何 GPU 图形设备。
- `Gpu`：要求严格使用显卡执行渲染。启动失败、遇到不受支持的内容、显卡设备丢失或执行故障均作为显式错误抛出，绝不悄然降级为 CPU。
- 软渲染适配器（Software GPU adapters）需要显式授权 `allow_software: true`。其测量耗时不能代表真实物理 GPU 的性能。

`RenderContext` 管理资产与结构化配额，不管理 GPU。`RenderOptions.gpu_context` 负责显式注入共享显卡设备，但不改变后端选择策略。

## FrameBuffer 像素缓冲区

`FrameBuffer` 是统一的内存图像存储类型；在直通 `SrgbRgba8` 格式下提供透明画面与 `save_png` 保存能力。`Composition::render_frame` 与 `FrameRenderer::render_frame` 返回不透明的（纯白底色，除非设置了 `RenderOptions::background`）直通 `SrgbRgba8` `FrameBuffer`。其 `write_ppm` 直接写入磁盘文件，不提供类似导出流水线那样的防覆盖事务保护。

| `PixelFormat` 像素格式 | 核心规范与使用约束 |
| --- | --- |
| `SrgbRgba8` | 显式指定直通（Straight）或预乘（Premultiplied）Alpha 通道 |
| 线性 RGBA16F | 线性预乘；色彩动态范围受半精度浮点限制 |
| 线性 RGBA32F | 线性预乘，无量化损失；通过 `linear_pixel` 读取，通过 `to_format` 转换 |
| `Yuv420` 8/10-bit | 受限范围 BT.709；分辨率宽高必须为偶数；严禁包含透明像素 |

切勿根据格式名称主观臆测内存行跨度（Stride），也严禁将不同 Alpha 表达形式的字节切片当作相同格式错误混淆。YUV 绝非任意 RGBA 缓冲区的代名词，HDR 也不提供自动色调映射。

## PreparedCpuFrame 与各级缓存

准备阶段在 CPU 侧负责生成矢量曲线、覆盖率、画刷记录、裁剪与滤镜数据。GPU 合成消费完全相同的经准备产物。

`ValidatedDisplayList` 在生成时已包含完整的结构测量信息，因此常规准备阶段无需重新扫描合法的指令流。准备就绪的图像画刷直接引用不可变的 `ImagePixels`，不存在二次延迟加载环节。

`RenderCache` 拥有独立限额的覆盖率、段落、字形、渐变与公式缓存分区。克隆的 `RenderOptions` 自动共享这些纯计算缓存。`SceneCache` 缓存创作期计算结果；Studio 的最终视口预览帧缓存属于更高一级的独立图层。

固定资产、字体与缓存可在多个会话间共享，而每个单次请求依然保留各自独立的诊断信息、渲染目标与预算边界。命中缓存既不能跳过参数校验，也不代表渲染已经执行完毕。

## GPU 纹理零拷贝输出

GPU 内部工作图像全程保留在 RGBA32F 浮点纹理中。暂存区数据上传、实例元数据打包与有限状态校验依然涉及 CPU 计算。纹理直接呈现并不代表 CPU 开销为零。

外部传入的目标纹理必须归属于同一个物理显卡设备，并严格满足声明格式的 2D、单图层、单 Mipmap、单采样渲染挂载目标（Render Attachment）契约。RGBA8/BGRA8 为编码输出格式；线性 RGBA16F 拥有独立的输出格式契约。

`FrameRenderer::submit_to_texture` 与 `GpuContext::submit_texture` 返回 `PendingTextureFrame` 异步凭证句柄。调用 `finish` 会严格校验命令提交状态、有限数值、OOM 内存溢出与设备健康度，全部通过后方可对外发布目标纹理。`is_ready` 支持轮询当前完成状态。丢弃未完成的任务绝不能把在途的在用显存当作空闲池条目错误回收。

Studio 与 eframe 共享同一个显卡设备并直接注册纹理，彻底消除了全帧 CPU 回读与二次上传。有限值校验依然可能涉及微小状态字的异步回读。返回 CPU 可读位图或内存帧的请求必须跨越其专属的回读与格式转换边界。

## 预算控制与错误防御

内存渲染目标仅在当前帧成功渲染完毕后才会原子替换已有数据；旧数据的内存在逻辑准入时全额计费。外部 GPU 纹理由于是就地绘制，在遇到硬件崩溃时无法提供类似文件导出那样的自动回滚保证，因此在完成检查通过前绝不能提前发布。

画质等级、像素分辨率、滤镜光晕尺寸、目标像素格式与外部引用都会直接影响资源消耗。`RenderQuality::Draft` 属于显式声明的快速近似策略，绝非悄然降低生产级 Final 的默认配置。

## 常见踩坑点

- 每一帧重新创建 `GpuContext` 会彻底破坏底层设备、内存池与着色器流水线的复用。
- 相同型号的显卡硬件并不意味着外部纹理必定属于同一个逻辑设备实例。
- 命令刚提交至 GPU 就立即显示或覆写纹理，会绕过底层的任务完成校验引发画面撕裂。
- 内存输出（Memory Output）与零拷贝纹理输出（Texture Output）有本质区别，前者涉及回读开销。
- 纯计算缓存、活跃帧显存、空闲对象池、外部强引用与系统物理 RSS 内存是截然不同的量纲。

## 后续章节

下一章：[10 音频处理](10-audio.md)。若不涉及音频，可直接跳至 [11 视频与动图导出](11-export.md)。
设计契约与底层源码参考：[渲染与导出设计](../../zh-CN/design/rendering.md)、[FrameRenderer 实现](../../../crates/motion-render/src/composition/frame_renderer.rs)。
