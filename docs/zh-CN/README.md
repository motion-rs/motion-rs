# 文档导航

## 学习与使用

- [教程指南](tutorial/README.md)：创作学习路径，从第一帧静帧到动画、内容、音频、导出与扩展。
- [Motion Studio](studio.md)：桌面工作台操作、预览与回放。
- [Gallery 展厅](../../gallery/README.md)：完整的示例短片作品。
- [命令行 CLI](cli.md)：`Project` 二进制的 `list`、`still`、`export`、`studio` 和 `schema` 子命令。
- [Rustdoc 源码](../../crates/motion/src/lib.rs)：精确的公开 API 签名与代码示例。

## 理解引擎设计

[设计契约](design/README.md)系统阐述了引擎的设计原则、分层架构、core/content 归属边界、渲染、导出、音频、资源管理、GPU 执行、粒子系统、可靠性以及声明式文档规范。它们是系统集成与贡献代码的权威参考，而非学习教程的前置要求。

## 维护与测量

- [开发指南](development.md)：功能特性矩阵、测试套件、测试固件与 CI 验证。
- [代码规范](style.md)：文件布局、字段顺序、注释与 lint 规则。
- [开发规划](plan.md)：项目优先级、待完成工作与性能优化规划。
- [性能基准](benchmarks.md)：基准工作负载、评测流程与当前实测结果。
- [基准测量清单](../performance-runs.json)：已记录的运行数据与环境溯源。
- [画质评测报告](quality-report.md)：图像质量评测结果与基准数据。

## 约定规范

文档中使用的专业术语在[设计词汇表](design/README.md#设计词汇表)中统一定义。教程、命令行页面以及渲染和资源设计页面中的 Rust 代码块均作为 Rustdoc 自动化测试进行编译验证（参见[开发指南](development.md)）。
