//! 操作：内嵌预览 / 缩略图提取（对应 ACR 的预览面板与 Adobe Bridge 缩略图来源）。
//!
//! DNG 文件中的预览以三种形式存在：
//! - JPEG 压缩的预览 IFD（`NewSubFileType != 0`，通常在 SubIFD 或链式 IFD）
//! - JXL 压缩的预览（DNG 1.7）
//! - IFD0 内嵌 TIFF 缩略图（未压缩 8 位 RGB/灰度，官方样例库全部是这种）
//!
//! JPEG/JXL 形式直接 dump 为独立文件；未压缩 8 位缩略图转写为 PPM；
//! 其他 RAW 形式（无损 JPEG / 高位深未压缩）不做解释器级转换，标记跳过。

use std::path::Path;

use serde::Serialize;

use dng_core::dng_tag_values::{ccJXL, ccJPEG, ccLossyJPEG, ccOldJPEG, ccUncompressed, piBlackIsZero, piRGB, piWhiteIsZero};

use crate::document::DngDocument;
use crate::error::DshResult;

/// 一个 IFD 的预览信息（列表视图）。
#[derive(Debug, Serialize)]
pub struct PreviewInfo {
    /// `info.ifd`（SubIFD 链）或 `info.chained_ifd` 中的下标。
    pub ifd_index: i32,
    /// "subifd" | "chained"。
    pub location: String,
    pub new_sub_file_type: u32,
    pub width: u32,
    pub height: u32,
    pub compression: String,
    /// 是否能直接产出独立图片文件。
    pub extractable: bool,
}

/// 列出所有非主图的 IFD（预览候选）。
pub fn list_previews(doc: &DngDocument) -> Vec<PreviewInfo> {
    let mut out = Vec::new();
    for (i, ifd) in doc.info.ifd.iter().enumerate() {
        if i as i32 == doc.info.main_index && ifd.new_sub_file_type == 0 {
            continue; // 主 RAW 图
        }
        out.push(preview_info(ifd, i as i32, "subifd"));
    }
    for (i, ifd) in doc.info.chained_ifd.iter().enumerate() {
        out.push(preview_info(ifd, i as i32, "chained"));
    }
    out
}

/// 提取策略。
enum ExtractKind {
    /// 原始字节直接 dump（JPEG/JXL 容器内嵌的就是完整图片文件）。
    RawDump(&'static str),
    /// 未压缩 8 位图转写 PPM（invert 用于 piWhiteIsZero）。
    Ppm { invert: bool, samples: u32 },
}

fn extract_kind(ifd: &dng_core::dng_ifd::DngIfd) -> Option<ExtractKind> {
    match ifd.compression {
        ccJPEG | ccOldJPEG => Some(ExtractKind::RawDump("jpg")),
        ccJXL => Some(ExtractKind::RawDump("jxl")),
        ccUncompressed => {
            let spp = ifd.samples_per_pixel;
            let bits_ok = (0..spp.min(4) as usize).all(|i| ifd.bits_per_sample[i] == 8);
            let chunky = ifd.planar_configuration == 1;
            let photo_ok = matches!(ifd.photometric_interpretation, piWhiteIsZero | piBlackIsZero | piRGB);
            if bits_ok && chunky && photo_ok && (spp == 1 || spp == 3) {
                Some(ExtractKind::Ppm {
                    invert: ifd.photometric_interpretation == piWhiteIsZero,
                    samples: spp,
                })
            } else {
                None
            }
        }
        _ => None,
    }
}

fn preview_info(ifd: &dng_core::dng_ifd::DngIfd, index: i32, location: &str) -> PreviewInfo {
    let compression = ifd.compression;
    let extractable = extract_kind(ifd).is_some();
    PreviewInfo {
        ifd_index: index,
        location: location.to_string(),
        new_sub_file_type: ifd.new_sub_file_type,
        width: ifd.image_width,
        height: ifd.image_length,
        compression: compression_name(compression),
        extractable,
    }
}

fn compression_name(c: u32) -> String {
    match c {
        ccUncompressed => "Uncompressed".into(),
        ccOldJPEG => "OldJPEG".into(),
        ccJPEG => "JPEG".into(),
        ccLossyJPEG => "LosslessJPEG".into(),
        ccJXL => "JXL".into(),
        _ => format!("Compression({c})"),
    }
}

/// 单个提取出的预览。
#[derive(Debug, Serialize)]
pub struct ExtractedPreview {
    pub ifd_index: i32,
    pub location: String,
    pub width: u32,
    pub height: u32,
    pub compression: String,
    pub output: String,
    pub bytes: u64,
}

/// 因格式原因跳过的预览。
#[derive(Debug, Serialize)]
pub struct SkippedPreview {
    pub ifd_index: i32,
    pub location: String,
    pub compression: String,
    pub reason: String,
}

/// 提取报告。
#[derive(Debug, Serialize)]
pub struct PreviewExtractReport {
    pub file: String,
    pub extracted: Vec<ExtractedPreview>,
    pub skipped: Vec<SkippedPreview>,
}

/// 提取所有可提取的内嵌预览到 `out_dir`。
///
/// 文件名规则：`{stem}_{location}{index}_{w}x{h}.{jpg|jxl}`。
pub fn extract_previews(doc: &mut DngDocument, out_dir: impl AsRef<Path>) -> DshResult<PreviewExtractReport> {
    let out_dir = out_dir.as_ref();
    std::fs::create_dir_all(out_dir)?;
    let stem = doc
        .path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("dng")
        .to_string();

    let mut report = PreviewExtractReport {
        file: doc.path.display().to_string(),
        extracted: Vec::new(),
        skipped: Vec::new(),
    };

    let candidates: Vec<(i32, &str)> = {
        let mut v: Vec<(i32, &str)> = (0..doc.info.ifd.len() as i32)
            .filter(|&i| !(i == doc.info.main_index && doc.info.ifd[i as usize].new_sub_file_type == 0))
            .map(|i| (i, "subifd"))
            .collect();
        v.extend((0..doc.info.chained_ifd.len() as i32).map(|i| (i, "chained")));
        v
    };

    for (index, location) in candidates {
        let ifd = if location == "subifd" {
            &doc.info.ifd[index as usize]
        } else {
            &doc.info.chained_ifd[index as usize]
        };

        let Some(kind) = extract_kind(ifd) else {
            report.skipped.push(SkippedPreview {
                ifd_index: index,
                location: location.to_string(),
                compression: compression_name(ifd.compression),
                reason: "非独立图片格式（RAW 形式），需解码管线".into(),
            });
            continue;
        };
        let ext = match &kind {
            ExtractKind::RawDump(e) => *e,
            ExtractKind::Ppm { .. } => "ppm",
        };

        // 数据来源：优先 JPEGInterchangeFormat（单块），否则拼接 strips/tiles
        let bytes = if ifd.jpeg_interchange_format != 0 && ifd.jpeg_interchange_format_length > 0 {
            doc.stream.set_read_position(ifd.jpeg_interchange_format)?;
            doc.stream.get(ifd.jpeg_interchange_format_length)?
        } else {
            let n = (ifd.tile_offsets_count as usize).min(ifd.tile_offset.len());
            let mut all = Vec::new();
            let mut failed = false;
            for i in 0..n {
                let count = ifd.tile_byte_count[i];
                if count == 0 {
                    continue;
                }
                doc.stream.set_read_position(ifd.tile_offset[i])?;
                match doc.stream.get(count as u32) {
                    Ok(chunk) => all.extend_from_slice(&chunk),
                    Err(_) => {
                        failed = true;
                        break;
                    }
                }
            }
            if failed {
                report.skipped.push(SkippedPreview {
                    ifd_index: index,
                    location: location.to_string(),
                    compression: compression_name(ifd.compression),
                    reason: "数据读取失败".into(),
                });
                continue;
            }
            all
        };

        if bytes.is_empty() {
            report.skipped.push(SkippedPreview {
                ifd_index: index,
                location: location.to_string(),
                compression: compression_name(ifd.compression),
                reason: "无数据".into(),
            });
            continue;
        }

        let out_path = out_dir.join(format!(
            "{stem}_{location}{index}_{}x{}.{}",
            ifd.image_width, ifd.image_length, ext
        ));

        let written = match kind {
            ExtractKind::RawDump(_) => {
                std::fs::write(&out_path, &bytes)?;
                bytes.len() as u64
            }
            ExtractKind::Ppm { invert, samples } => {
                // 未压缩缩略图 → PPM：截取 w*h*spp（TIFF strip 行内无填充）
                let need = (ifd.image_width as usize)
                    * (ifd.image_length as usize)
                    * samples as usize;
                if bytes.len() < need {
                    report.skipped.push(SkippedPreview {
                        ifd_index: index,
                        location: location.to_string(),
                        compression: compression_name(ifd.compression),
                        reason: format!("数据不足: {} < {}", bytes.len(), need),
                    });
                    continue;
                }
                let mut data = bytes[..need].to_vec();
                if invert {
                    data.iter_mut().for_each(|b| *b = !*b);
                }
                let header = if samples == 3 {
                    format!("P6\n{} {}\n255\n", ifd.image_width, ifd.image_length)
                } else {
                    format!("P5\n{} {}\n255\n", ifd.image_width, ifd.image_length)
                };
                let mut out_bytes = header.into_bytes();
                out_bytes.extend_from_slice(&data);
                std::fs::write(&out_path, &out_bytes)?;
                out_bytes.len() as u64
            }
        };

        report.extracted.push(ExtractedPreview {
            ifd_index: index,
            location: location.to_string(),
            width: ifd.image_width,
            height: ifd.image_length,
            compression: compression_name(ifd.compression),
            output: out_path.display().to_string(),
            bytes: written,
        });
    }

    Ok(report)
}
