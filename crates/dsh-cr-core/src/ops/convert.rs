//! 操作：DNG Converter（对应 Adobe DNG Converter 的转码子集）。
//!
//! 与 `ops::metadata::save_as_dng` 共享写出管线，但面向「转换」场景暴露
//! ACR Converter 的参数面：压缩策略、向后兼容版本、嵌入 original raw。
//!
//! 已知限制（如实呈现，见 ARCHITECTURE.md §6）：
//! - E1：RustDNG writer 不写 OriginalRawFileData 三个 tag → `embed_original`
//!   目前返回 `Unimplemented`，不做静默降级。
//! - 转换会重编码像素（无损压缩）：u16 → 无损 JPEG，f32 → Deflate，
//!   u8 → 未压缩；这与 write_dng 内部策略一致。

use std::path::Path;

use serde::{Deserialize, Serialize};

use dng_core::dng_tag_values::dngVersion_SaveDefault;

use crate::document::DngDocument;
use crate::error::{DshError, DshResult};
use crate::ops::metadata;

/// 转换选项。
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ConvertOptions {
    /// "auto"（默认，按像素类型选无损压缩）| "uncompressed"。
    pub compression: Option<String>,
    /// 向后兼容版本："1.1" | "1.4" | "1.6" | "1.7" | "save"（默认 save）。
    pub backward_version: Option<String>,
    /// 嵌入 original raw（缺口 E1：RustDNG 写侧未实现 → Unimplemented）。
    pub embed_original: Option<bool>,
}

impl Default for ConvertOptions {
    fn default() -> Self {
        ConvertOptions {
            compression: None,
            backward_version: None,
            embed_original: None,
        }
    }
}

/// 转换报告。
#[derive(Debug, Serialize)]
pub struct ConvertReport {
    pub input: String,
    pub output: String,
    pub file_size_bytes: u64,
    /// 实际使用的压缩："lossless_jpeg" | "deflate" | "uncompressed" | "auto"。
    pub compression: String,
    /// 写出的 DNG 版本。
    pub dng_version: String,
}

/// 把 DNG 转换/另存为另一个 DNG（压缩与版本可调）。
pub fn convert_to(
    doc: &mut DngDocument,
    out_path: impl AsRef<Path>,
    opts: &ConvertOptions,
) -> DshResult<ConvertReport> {
    let compression = opts.compression.as_deref().unwrap_or("auto");
    if !matches!(compression, "auto" | "uncompressed") {
        return Err(DshError::BadRequest(format!(
            "未知压缩模式: {compression}（支持 auto/uncompressed）"
        )));
    }
    if opts.embed_original.unwrap_or(false) {
        return Err(DshError::Unimplemented(
            "嵌入 original raw：RustDNG writer 未实现 OriginalRawFileData 三 tag 的写 \
             入（缺口 E1，ARCHITECTURE.md §6）。转换照常进行可去掉 embed_original。"
                .into(),
        ));
    }

    let version = match opts.backward_version.as_deref().unwrap_or("save") {
        "1.1" => Some(dng_core::dng_tag_values::dngVersion_1_1_0_0),
        "1.4" => Some(dng_core::dng_tag_values::dngVersion_1_4_0_0),
        "1.6" => Some(dng_core::dng_tag_values::dngVersion_1_6_0_0),
        "1.7" => Some(dng_core::dng_tag_values::dngVersion_1_7_0_0),
        "save" => None,
        other => {
            return Err(DshError::BadRequest(format!(
                "未知 backward version: {other}（支持 1.1/1.4/1.6/1.7/save）"
            )))
        }
    };

    let uncompressed = compression == "uncompressed";

    // 报告实际使用的压缩：写策略由像素类型决定（与 writer 内部一致），
    // 这里按同一规则从 IFD 元数据推算（不额外读像素）。注意 IFD 的
    // pixel_type() 不区分 float——浮点看 sample_format。
    let raw_ifd = (doc.info.main_index >= 0)
        .then(|| &doc.info.ifd[doc.info.main_index as usize]);
    let is_float = raw_ifd
        .map(|ifd| ifd.sample_format[0] == 3)
        .unwrap_or(false);
    let is_long = raw_ifd.map(|ifd| ifd.bits_per_sample[0] > 16).unwrap_or(false);

    let save_opts = metadata::SaveOptions {
        uncompressed,
        max_backward_version: version,
    };

    let save = metadata::save_as_dng(doc, out_path, &save_opts)?;

    let actual = if uncompressed {
        "uncompressed".to_string()
    } else if is_float {
        "deflate".to_string()
    } else if is_long {
        "uncompressed".to_string()
    } else {
        "lossless_jpeg".to_string()
    };

    // 读回产物的 DNG 版本。
    let version_str = {
        let reopened = DngDocument::open(&save.output)?;
        crate::ops::info::format_dng_version(reopened.info.shared.dng_version)
    };

    Ok(ConvertReport {
        input: doc.path.display().to_string(),
        output: save.output,
        file_size_bytes: save.file_size_bytes,
        compression: actual,
        dng_version: version_str,
    })
}

/// 便捷入口：直接从文件转换（内部 open → convert_to）。
pub fn convert_file(
    in_path: impl AsRef<Path>,
    out_path: impl AsRef<Path>,
    opts: &ConvertOptions,
) -> DshResult<ConvertReport> {
    let mut doc = DngDocument::open(in_path)?;
    convert_to(&mut doc, out_path, opts)
}

/// dngVersion_SaveDefault 供 CLI/宿主使用。
pub fn save_default_version() -> u32 {
    dngVersion_SaveDefault
}
