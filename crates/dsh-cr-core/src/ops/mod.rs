//! ACR 式 DNG 操作集。
//!
//! 每个操作对应插件 ABI 里的一个 op；操作函数一律
//! `fn xxx(&mut DngDocument, ...) -> DshResult<报告结构体>`，
//! 报告结构体均实现 `serde::Serialize`（插件 ABI 用 JSON 传输）。

pub mod convert;
pub mod info;
pub mod metadata;
pub mod preview;
pub mod render;
pub mod validate;
