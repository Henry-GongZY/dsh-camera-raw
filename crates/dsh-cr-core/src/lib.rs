//! # DSH Camera Raw — 插件核心引擎
//!
//! 复刻 Adobe Camera Raw 的主要 DNG 操作功能，底层依赖
//! [RustDNG]（Adobe DNG SDK 1.7.1 的 100% 纯 Rust 重写）。
//!
//! 设计契约见仓库根目录 `ARCHITECTURE.md`，执行台账见 `PROGRESS.md`。
//!
//! [RustDNG]: https://github.com/Henry-GongZY/RustDNG

pub mod document;
pub mod error;
pub mod ops;

pub use document::DngDocument;
pub use error::{DshError, DshResult};

/// 解析向后兼容版本字符串（CLI / 插件 ABI / convert 共用）。
/// "1.1"|"1.4"|"1.6"|"1.7" → Some(Some(v))；"save"/"auto"/None → Some(None)；
/// 未知 → Err。
pub fn backward_version_from_str(
    s: Option<&str>,
) -> DshResult<Option<u32>> {
    use dng_core::dng_tag_values::{
        dngVersion_1_1_0_0, dngVersion_1_4_0_0, dngVersion_1_6_0_0, dngVersion_1_7_0_0,
    };
    match s.unwrap_or("save") {
        "1.1" => Ok(Some(dngVersion_1_1_0_0)),
        "1.4" => Ok(Some(dngVersion_1_4_0_0)),
        "1.6" => Ok(Some(dngVersion_1_6_0_0)),
        "1.7" => Ok(Some(dngVersion_1_7_0_0)),
        "save" | "auto" => Ok(None),
        other => Err(DshError::BadRequest(format!(
            "未知 backward version: {other}（支持 1.1/1.4/1.6/1.7/save）"
        ))),
    }
}
