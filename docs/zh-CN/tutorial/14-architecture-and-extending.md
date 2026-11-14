# 14 架构与扩展

## 本章学习目标

为业务需求选用最精准的扩展层级，深入理解值对象、多媒体内容、渲染底层、视频编码以及桌面工作台的所有权归属，杜绝将 GPU 硬件设备或磁盘 I/O 错误混入核心 Core 层的设计倒退。
最小的 `Drawable` 扩展无需任何可选特性。内容与原生编解码扩展需通过其所属的 Crate 选择对应特性。

## 最小可运行示例：自定义 Drawable

```rust,no_run
use motion::prelude::*;
use motion::extend::{Canvas, DrawContext, Drawable, ValidationError};

struct Badge;

impl Drawable for Badge {
    fn bounds(&self, _context: &DrawContext) -> Result<Rect, ValidationError> {
        Ok(Rect::new(0.0, 0.0, 160.0, 60.0))
    }

    fn draw(
        &self,
        context: &DrawContext,
        canvas: &mut Canvas<'_>,
    ) -> Result<(), ValidationError> {
        canvas.fill_rect(self.bounds(context)?, Color::rgb(20, 150, 125))?;
        canvas.fill_rect(Rect::new(20.0, 20.0, 140.0, 40.0), Color::WHITE)
    }
}

fn main() -> motion::Result {
    let timeline = Timeline::new(Size::new(320.0, 180.0), fps!(30)).track(|track| {
        track.scene(1.seconds(), |_| Node::from(Badge))
    }).build()?;
    render_still(&timeline, Time::ZERO, &RenderOptions::default())?
        .save_png("tutorial-drawable.png")
}
```

`Drawable` 负责返回局部包围盒并向经校验的 `Canvas` 录制指令。它绝不创建硬件设备、不执行磁盘写入，也不返回具有独立后端生命周期的异步图像任务。`bounds` 方法返回 Result，因此 Drawable 会将非法几何作为 `ValidationError` 明确抛出。它仅适用于真正的叶子节点。若内容需要在当前时刻动态展开为其他节点树（如动画文本、SVG 与数学公式），应当实现 `DeferredContent::resolve` 并通过 `Node::deferred` 包装。文本、SVG 与公式对此共享统一的流水线：由不可变编译键创建的 `CompileCache`、编译出的矢量树、`VectorSelection` 以及 `VectorTree::reveal`；请直接复用该流水线，切勿手动清空缓存。

## Crate 职责与依赖方向

| Crate 包名 | 职责范围与依赖方向约束 |
| --- | --- |
| `motion-core` | 值对象、动画系统、创作抽象、绘制指令、资源契约、音频事件；绝不依赖任何上游引擎 crate |
| `motion-render` | 覆盖率计算、CPU/GPU 双后端、渲染会话、帧目标；依赖 core |
| `motion-content` | 图像、排版布局、SVG、矢量、粒子及门控内容；依赖 core |
| `motion-formula` | Typst 文档排版与数学公式计算；依赖 core 和 content |
| `motion-encode` | PCM 准备、流式混音、事务性视频编码；依赖 core 和 render，可选依赖 FFmpeg |
| `motion-scene` | 声明式 JSON 文档与 Schema 规范；依赖 core、render 和可选 content |
| `motion-studio` | 桌面工作台预览、实时回放与导出编排；依赖 core、render 和 encode |
| `motion-rs` | 顶层 Facade 统一门面、Project/CLI；按特性条件重新导出 |
| `motion-ffmpeg` | 原生 C ABI 绑定及线程隔离的底层资源所有权；无上游依赖 |
| `motion-bench` / `motion-gallery` | 性能基准与短片作品，非核心引擎实现 |

每个类型在唯一的归属 crate 中定义；底层包绝不反向依赖高层的便利命名空间。Core 中的图像、实例与字体快照协议绝不初始化外部内容编译器、显卡设备或桌面窗口。

## 五大公开导入分类

| 命名空间 | 职责与包含的内容 |
| --- | --- |
| `motion::prelude` | 仅面向日常创作的值对象、形状与表达式；不含编码器、进度接收器或渲染中间表示 |
| `motion::extend` | 面向系统扩展的协议 Trait，如 `Drawable`、`Asset`、`AudioSource`、`RenderPrepare`、`Encoder` |
| `motion::render` | 显式底层绘制指令、配额预算、渲染会话、像素格式、GPU 上下文 |
| `motion::encode` | 编码器与其工厂闭包、进度监听器、`ExportReport`、`ExportPhase`、`NoProgress` |
| `motion::eager` | 预编译的矢量/公式对象及其实时求值操作 |

Facade 直接重新导出共享类型，不增加额外的包装胶水层。内部模块严禁通过公开的 prelude 链式导入，底层 crate 也不盲目向下重新导出上游的所有便利类型。

## 扩展点的精准选择

| 扩展点 Trait | 契约与职责 |
| --- | --- |
| `Drawable` | 测量局部包围盒并向 Canvas 发射合法指令，失败时绝不产生部分成功输出 |
| `Asset` / `AssetsExt` | 不可变快照、带类型的延迟句柄、常驻字节计量、显式资源所有者 |
| `AudioSource` | 纯函数式随机寻址音频采样、声道/采样率声明、有界读取块 |
| `RenderPrepare` | 将已校验的绘制数据准备为渲染后端可消费的图元记录 |
| `Encoder` | 声明支持的像素格式与硬件能力，严格保序 open/write/finish，自理输出事务与 Drop 清理 |
| `ProgressSink` | 线程安全的进度通知与协作式取消；完成帧数不代表文件已提交 |
| 粒子实例协议 | 有界的随机寻址粒子状态采样与样式映射 |

`AssetsExt` 是 content 包提供的便利 trait，在开启对应特性时提供带类型的资源访问接口。真实的句柄类型为 `Handle<A>`，而非早期草案中的 `AssetHandle`。

在预加载 `Arc<A>` 时，请准确实现并上报 `Asset::retained_bytes`。在执行期，请通过 `get_with` 将 `DrawContext` 中的诊断收集器显式传递给捕获的句柄，而非依赖已被彻底移除的全局线程局部变量。

## 保持显式清晰的 GPU 所有权

`RenderContext` 管理资产与结构化策略。GPU 状态必须通过 `RenderOptions.gpu_context` 或显式的上下文 API 显式注入。在将纹理交付给 UI 框架之前，必须完整执行异步完成检查；外部持有的句柄独立于请求与缓存配额之外。

引擎当前不存在所谓的 `ImageProducer`、`ProducedImage` 或 3D 模型扩展流水线。切勿重新引入历史设计审查中已被彻底废弃的陈旧接口。图像画刷直接消费不可变的 `ImagePixels`；外部纹理属于渲染输出层面的契约。

## 代码贡献与边界规范

将功能实现严格保留在直接控制其行为的模块中。测试代码紧随实现代码，大型测试套件置于相邻的测试模块中。建议从最小特性配置开始检查非测试代码的编译，避免测试专有依赖或工作区的特性合并不慎掩盖了依赖缺失。

请独立验证 `vector`、`particles`、`ink`、`diagram`、`captions`、`typst` 与 `formula` 特性，而非仅测试默认特性或全量特性。原生 FFmpeg、GPU 与桌面窗口拥有额外的硬件与系统库要求；特性编译通过不等于运行时环境健康。

运行结构检查工具校验文件规范：

```sh
node tools/check-structure.cjs
```

完整的编译、测试、Rustdoc 以及真实硬件设备门禁参见[开发指南](../development.md)。严禁将某个局部的代码改动混杂在全库批量格式化、不相关的重构或新增 facade 包装层中提交。

## 常见踩坑点

- 试图把底层显卡渲染设备或 Typst 内部编译器细节塞入 core 包的公开契约中。
- 在内部实现模块中使用 facade 或公开 prelude 聚合内部引用。
- 仅依赖全特性（all-features）测试来验证各个独立的子包特性组合。
- 在编写自定义 `Encoder` 时忽视失败或 Drop 时的清理工作，却误以为引擎底层会自动保护目标文件。
- 脱离当前最新源码，盲目从旧设计草案中复制代码片段。

## 后续章节

将上述知识应用于实际短片创作：[Gallery 展厅指南](appendix-gallery.md)。返回[教程索引目录](README.md)。
设计契约与底层规范参考：[系统架构与分包设计](../../zh-CN/design/architecture.md)、[核心设计原则](../../zh-CN/design/README.md)。
协议源码：[Drawable 协议](../../../crates/motion-core/src/drawing/context.rs)、[Encoder 协议](../../../crates/motion-encode/src/export.rs)。
