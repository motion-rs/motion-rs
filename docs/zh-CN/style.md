# 代码风格与规范

本工作区中 Rust 源码的编码规范约定。标记为 **(CI)** 的规则会在 Structure 工作流中通过 `tools/check-structure.cjs` 或 `tools/rust-format.cjs --check` 自动化检查；其余规则为代码审查约定。验证命令详见[开发与维护检查](development.md)。

`.github/copilot-instructions.md` 与 `.github/instructions/` 中的 AI 编程指令同样遵循这些规则，修改时请保持同步。

## 格式化

- 遵循仓库的 `rustfmt.toml` 执行 `cargo fmt`（Rust 2024 版本）。**(CI)**
- `node tools/rust-format.cjs` 在各个顶层项目（item）之间添加一行空行。连续的 `use` 声明以及连续的 `mod name;` 声明保持紧凑相连，以便 rustfmt 对每个分组进行独立排序。**(CI)**

## 文件布局规范

每个非测试源文件严格遵循以下顺序结构：

```text
//! 连续的模块概括摘要块。                       (CI: 必须存在且连续)
use …;                                          (CI: 仅限文件头部)
mod …; / pub mod …; pub use …;                  (CI: 仅限文件头部)
const / static
公开类型 A
    impl A { 构造器 → 访问器 → Builder → 业务操作 }
    impl Trait for A { … }                      // trait 实现紧随类型其后，包含 Drop
公开类型 B …
公开自由函数
私有辅助类型与函数
#[cfg(test)] mod tests { … }                    // 始终置于文件最末尾 (CI)
```

- 模块级文档摘要 `//!` 必须以大写字母开头，阐述该文件的核心职责，而非描述导入了哪些模块。**(CI)**
- 独立于外部文件的 `#[cfg(test)] mod tests;` 声明可置于文件头部声明区域。
- 仅有一个调用者的私有辅助函数应紧跟在该调用者之后定义。
- 实现源码文件严格限制在 800 行以内。测试文件（`tests.rs`、`*_tests.rs`、`tests/` 目录）不受此限制。**(CI)**

## 模块组织

- 使用 `foo.rs` 搭配同名目录 `foo/`；严禁新增 `mod.rs` 文件。
- 拆分文件时应按职责拆解，而非仅按代码行数切分；原文件保留为重新导出符号的 facade，以确保公开导出路径稳定不变。
- 300 行以内的内联测试可保留在文件末尾。更大规模的测试套件应移至 `foo/tests.rs`，并在文件头部声明 `#[cfg(test)] mod tests;`。
- 除测试模块和 prelude 外，避免使用通配符导入（glob import）。

## 类型与字段规范

- 结构体字段顺序遵循：必需输入参数 → 沿数据流顺序的可选配置项 → 派生数据、缓存或运行时状态。
- `Default` 实现、构造函数和 Builder 方法中字段的罗列顺序与结构体声明顺序保持一致。若调整字面量顺序会改变带副作用初始化的求值顺序，应先通过 `let` 绑定提取变量。
- 仅无不变式约束的纯数据类型（如 `Point`、`Rect`）才公开公有字段。具有状态不变式约束的类型一律保持字段私有，通过构造器和访问器对外暴露。严禁新增公有元组结构体，一律使用命名字段。
- `#[repr(C)]` 以及 GPU ABI 结构体严格遵循 Shader WGSL 内存布局。保持 Rust 主机侧与 WGSL 侧的名称完全一致，并在编译期通过断言校验结构体大小与字段偏移量。
- `match` 分支在可行时遵循枚举声明顺序。

## 构造函数与构建模式

- `new` / `try_new` 仅接收必需输入参数，对其余字段应用合理的默认值。
- `builder()` 是配置复杂参数的通道：通过链式方法指定可选配置，最后调用 `build()` 执行一次性校验（如 `Spring`、`Noise`、`Oscillator`、`BrushStroke`、`Path2D`）。
- 仅有必需输入的类型不提供 Builder。新手示例优先使用 `new` / `try_new`；仅当需要修改默认参数时才使用 `builder()`。
- `Codec` 通过预设遵循相同规则（如 `Codec::h265(18)`），并通过链式方法精细配置（`.ten_bit()`、`.with_preset(..)`）。

## 错误处理与 Panic 约定

- 所有可能失败的操作统一返回 `Result<_, ValidationError>`（或具体的 crate 错误类型）。
- 新增 API 仅提供返回 Result 的易失操作形式。现存的便利性 panic 函数统一单行委托给其对应的 `try_*` 形式，并在文档注释的 `# Panics` 章节下详细记录触发条件。
- 生产环境中的每个 `expect` 都必须在 `tools/check-expects.cjs` 中登记具体原因，且登记的消息必须在代码中确实存在。生产代码中严禁出现裸露的 `unwrap()`，也不允许未携带阐明不变式消息的 `unreachable!()`。**(CI)**
- 加锁 `Mutex` 或等待 `Condvar` 时必须使用 `motion_core::sync`（`lock_unpoisoned`、`wait_unpoisoned`），以便在锁中毒时能够使用纯数据值安全继续执行，而非引发二次 panic；严禁直接对锁操作调用 `expect`。

## 常量定义

在验证逻辑中使用的限制阈值、协议魔数和预算单位，必须定义为具名 `const` 并附带简要文档注释，严禁在逻辑中硬编码字面量。

## 注释规范

- 注释仅用于解释不变式约束、接口契约与非显而易见的设计意图；切勿简单复述代码表面逻辑。
- 每个 `unsafe` 代码块必须附带 `// SAFETY:` 注释说明其健全性保证；每个 `unsafe fn` 必须包含 `# Safety` 文档章节（由 `clippy::undocumented_unsafe_blocks` 与 `clippy::missing_safety_doc` 检查）。
- 文档注释必须是完整大写字母开头的英文句子；通过 ``[`Type`]`` 链接具体类型。
- 严禁在代码中遗留 `TODO` 或 `FIXME` 注释；如有待办请创建对应的 GitHub Issue。
- WGSL 着色器遵循[着色器规范](../../.github/instructions/wgsl.instructions.md)。

## Lint 规则

- 除 `motion-ffmpeg` 开启 `unsafe_op_in_unsafe_fn` 检查外，所有库 crate 一律声明 `#![forbid(unsafe_code)]`。
- 根目录 `Cargo.toml` 中的工作区 lints 通过 `lints.workspace = true` 统一应用于所有 package。
- 在抑制警告时，优先在最窄的作用域内使用 `#[expect(lint)]` 代替 `#[allow(lint)]`，以便在压制规则过时时能被编译器即时告警。
