# 08 Typst 与数学公式

## 本章学习目标

将高质量的数学公式排版无缝集成到共享的矢量场景流水线中，并进一步实现数学符号拾取选择、公式动态书写以及跨公式的几何 Morph 变形。
单页常规排版需要 `typst` 特性。数学公式创作、符号选择与公式 Morph 需要 `formula` 特性（它隐式包含前者）。

## 最小可运行示例

在依赖中开启 `formula` 特性。公式编译器使用显式的纯内存沙箱环境与内嵌字体；无需在宿主系统中安装外部 `typst` 命令行。

```rust,no_run
use motion::eager::FormulaCompiler;
use motion::prelude::*;

fn main() -> motion::Result {
    let compiler = FormulaCompiler::new();
    let formula = compiler.compile(
        "E = m c^2",
        &FormulaOptions {
            size: 40.0,
            fill: Color::rgb(20, 140, 120),
            ..FormulaOptions::default()
        },
    )?;
    let selected = formula
        .select_symbol("c")?
        .apply(|node| node.translate(0.0, -8.0));
    let timeline = Timeline::new(Size::new(480.0, 180.0), fps!(30)).track(|track| {
        track.scene(1.seconds(), move |_| selected.clone().translate(40.0, 55.0))
    }).build()?;
    render_still(&timeline, Time::ZERO, &RenderOptions::default())?
        .save_png("tutorial-formula.png")
}
```

编译与符号选择在场景构建前即完成校验。后续渲染所有帧均可直接复用该准备好的矢量产物。

## 接口形式选择指南

| API 接口 | 核心功能与执行时机 |
| --- | --- |
| `TypstCompiler::compile` | 将受信任的单页 Typst 标记编译为 `TypstDocument` |
| `TypstDocument::node` | 将轮廓化矢量 SVG 页面挂入共享矢量流水线 |
| `FormulaCompiler::compile` | 提前即时编译生成数学 `FormulaAsset` 与元数据 |
| `Formula::typst` | 在场景求值中声明延迟加载的数学公式描述 |
| `select_symbol` / `select_subexpression` | 从实际语法源中精准选择单个符号或子表达式 |
| `write` | 根据 0..=1 进度平滑展开矢量公式手写动画 |
| `morph_to` / `FormulaMorph::sample` | 在两个公式间建立符号对应关系并采样 Morph 变形 |

数学公式输入严格采用 Typst 数学语法，绝非隐式 LaTeX 翻译。切勿直接向编译器传入外部 LaTeX 宏包字符串。

## 延迟创作模式 (Deferred Authoring)

该模式同样需要 `formula` 特性，适用于无需在场景前提前读取数学符号元数据的常规动画：

```rust,no_run
use motion::prelude::*;

fn main() -> motion::Result {
    let equation = Formula::typst("f(t) = sin(2 pi t)")
        .size(36.0)
        .fill(Color::rgb(20, 140, 120));
    let timeline = Timeline::new(Size::new(480.0, 180.0), fps!(30)).track(|track| {
        track.scene(2.seconds(), move |context| {
            equation.clone().write(context.progress()).translate(32.0, 60.0)
        })
    }).build()?;
    render_still(&timeline, 1.seconds().after_origin(), &RenderOptions::default())?
        .save_png("tutorial-formula-writing.png")
}
```

`Formula` 保存源码与排版参数；语法错误在场景编译期会作为精确诊断信息反馈。`.write` 仅改变显现进度，绝不在每一帧重新初始化编译器。若修改了字体、源码、字号或颜色，系统会自动生成新缓存键。

## 符号选择与手写显现

符号选择严格基于 Typst 原始抽象语法树（AST）与渲染发生的位置，而非从最终字形坐标反向瞎猜。重复出现的符号会产生多个匹配项。区间选择采用左闭右开的符号索引范围，而非任意字符串切片下标。

公式书写是通过矢量弧长展开已排版好的完整轮廓，绝不是逐帧截断标记字符串。直接截断源码往往破坏 Typst 闭合语法导致解析报错，且重新排版会导致已展开字符发生剧烈排版跳动。预排版好的完整公式搭配进度展开是保持排版稳定一致的最佳做法。

## 跨公式 Morph 变形

在场景之外提前编译两端的公式，调用 `morph_to` 建立拓扑关联，随后在帧回调中通过 `sample` 采样进度。Gallery 中的经典短片 Harmonics 便采用了该模式：

```rust,no_run
use motion::eager::FormulaCompiler;
use motion::prelude::*;

fn main() -> motion::Result {
    let compiler = FormulaCompiler::new();
    let options = FormulaOptions::default();
    let first = compiler.compile("f(t) = sin(2 pi t)", &options)?;
    let second = compiler.compile("f(t) = sin(4 pi t)", &options)?;
    let morph = first.morph_to(&second)?;
    let _midpoint = morph.sample(0.5)?;
    Ok(())
}
```

这是矢量内容与数学符号之间的几何拓扑插值，绝非两张位图之间的简单交叉溶解（Crossfade）。创作者应自检符号对应关系与结构匹配度。对于结构差异过大的极端情形，建议使用淡入淡出或重新拆分公式表达式。

## 安全与信任模型

Typst 编译器使用显式的输入内容与内存字体，严禁访问任意外部磁盘文件、包导入或读取系统当前日期。限制单页文档、源码长度与输出大小并不等同于对编译 CPU 耗时、临时内存或内部 Memoize 缓存施加了绝对硬沙箱。

请仅编译受信任的代码内容。若在服务端接收不可信用户的任意排版源码，必须在操作系统进程层面做好隔离、资源限制与超时监控。库级别的长度校验不代表安全沙箱。

扩展项目字体时，请提供冻结的 `FontLibrary` 字节。后续追加注册的字体绝不能悄然修改先前的编译器实例或已有文档快照。

## 常见踩坑点

- 仅开启 `typst` 特性无法使用公式级 API。数学符号选择与 Morph 变形必须开启 `formula` 特性。
- 应当提前预编译或复用延迟对象，切勿在每一帧内部重新创建 `TypstCompiler` 或重新编译完全不变的源码。
- 字节下标、源码字符范围与渲染符号范围属于截然不同的索引概念，切勿混用。
- 源码长度上限无法替代防算力消耗的沙箱隔离。
- 公式在引擎中是活生生的纯矢量节点；其矢量路径完整参与常规的布局测量、空间变换、几何裁剪与图层混合。

## 后续章节

下一章：[09 渲染与合成](09-rendering.md)。
完整可运行程序：[Typst 公式示例](../../../examples/typst_formula.rs)。
范例短片：[Harmonics](../../../gallery/src/demos/harmonics.rs)。设计契约与底层实现：[编译器实现](../../../crates/motion-formula/src/compiler.rs)、[Typst 文档资产实现](../../../crates/motion-formula/src/typst_asset.rs)。
