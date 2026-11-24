//! 操作：ACR 式元数据编辑 + 另存 DNG。
//!
//! ACR / MWG 口径：**XMP 是元数据的单一事实来源**，EXIF 随同步更新。
//! 本实现的编辑顺序：
//! 1. 双边显式写入（EXIF 结构体字段 + XMP 属性，避免依赖上游同步映射的覆盖面）
//! 2. `synchronize_metadata()`（XMP→EXIF 同步 + IPTC Ingest + MWG 日期规则）
//! 3. 保存时由 writer 统一发射（EXIF IFD / XMP 包 / IPTC）
//!
//! 另存说明：RustDNG 写侧未实现 OriginalRawFileData 嵌入（缺口 E1，见
//! ARCHITECTURE.md §6），因此「编辑元数据后另存」会**重编码像素**
//! （未压缩或无损压缩），不是字节级无损——报告中如实标注。

use std::path::Path;

use serde::{Deserialize, Serialize};

use dng_core::dng_date_time::{DngDateTime, DngDateTimeInfo};
use dng_core::dng_file_stream::DngFileStream;
use dng_core::dng_tag_values::dngVersion_SaveDefault;
use dng_core::DngString;
use dng_sdk::dng_image_writer::DngImageWriter;
use dng_xmp::{XMP_NS_DC, XMP_NS_EXIF, XMP_NS_TIFF, XMP_NS_XMP};

use crate::document::DngDocument;
use crate::error::{DshError, DshResult};

/// 单条元数据编辑（JSON tag/content 风格，插件 ABI 与 CLI 共用）。
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "field", content = "value", rename_all = "snake_case")]
pub enum MetadataEdit {
    /// EXIF ImageDescription / XMP dc:description
    ImageDescription(String),
    /// EXIF Artist / XMP dc:creator（单项序列）
    Artist(String),
    /// EXIF Copyright / XMP dc:rights
    Copyright(String),
    /// EXIF Software / XMP xmp:CreatorTool
    Software(String),
    /// EXIF UserComment / XMP exif:UserComment
    UserComment(String),
    /// EXIF DateTimeOriginal / XMP exif:DateTimeOriginal。
    /// 接受 "YYYY:MM:DD HH:MM:SS" 或 ISO "YYYY-MM-DDTHH:MM:SS"。
    DateTimeOriginal(String),
    /// XMP xmp:Rating（ACR 星级 0-5）
    Rating(i32),
    /// XMP xmp:Label（ACR 颜色标签）
    Label(String),
}

/// 编辑结果报告（`after` 为同步后的实际读回值）。
#[derive(Debug, Serialize)]
pub struct MetadataEditReport {
    pub file: String,
    pub applied: Vec<String>,
    pub after: AfterValues,
}

#[derive(Debug, Serialize)]
pub struct AfterValues {
    pub image_description: String,
    pub artist: String,
    pub copyright: String,
    pub software: String,
    pub user_comment: String,
    pub date_time_original: Option<String>,
    pub rating: Option<String>,
    pub label: Option<String>,
}

/// 执行一批元数据编辑（内存态，不落盘；落盘用 [`save_as_dng`]）。
pub fn edit_metadata(doc: &mut DngDocument, edits: &[MetadataEdit]) -> DshResult<MetadataEditReport> {
    let mut applied = Vec::new();

    for edit in edits {
        match edit {
            MetadataEdit::ImageDescription(v) => {
                doc.negative.metadata_mut().get_exif_mut().image_description.set(v.as_bytes());
                doc.negative.metadata_mut().get_xmp_mut().set_string(
                    XMP_NS_DC,
                    "dc:description",
                    &DngString::from_bytes(v.as_bytes()),
                );
                applied.push("image_description".into());
            }
            MetadataEdit::Artist(v) => {
                doc.negative.metadata_mut().get_exif_mut().artist.set(v.as_bytes());
                let mut list = dng_core::dng_string_list::DngStringList::new();
                list.append(&DngString::from_bytes(v.as_bytes()));
                doc.negative.metadata_mut().get_xmp_mut().set_string_list(
                    XMP_NS_DC,
                    "dc:creator",
                    &list,
                    false,
                );
                applied.push("artist".into());
            }
            MetadataEdit::Copyright(v) => {
                doc.negative.metadata_mut().get_exif_mut().copyright.set(v.as_bytes());
                doc.negative.metadata_mut().get_xmp_mut().set_string(
                    XMP_NS_DC,
                    "dc:rights",
                    &DngString::from_bytes(v.as_bytes()),
                );
                applied.push("copyright".into());
            }
            MetadataEdit::Software(v) => {
                doc.negative.metadata_mut().get_exif_mut().software.set(v.as_bytes());
                doc.negative.metadata_mut().get_xmp_mut().set_string(
                    XMP_NS_XMP,
                    "xmp:CreatorTool",
                    &DngString::from_bytes(v.as_bytes()),
                );
                applied.push("software".into());
            }
            MetadataEdit::UserComment(v) => {
                doc.negative.metadata_mut().get_exif_mut().user_comment.set(v.as_bytes());
                doc.negative.metadata_mut().get_xmp_mut().set_string(
                    XMP_NS_EXIF,
                    "exif:UserComment",
                    &DngString::from_bytes(v.as_bytes()),
                );
                applied.push("user_comment".into());
            }
            MetadataEdit::DateTimeOriginal(v) => {
                let normalized = normalize_datetime(v)?;
                let mut dt = DngDateTime::new();
                if !dt.parse(normalized.as_bytes()) {
                    return Err(DshError::BadRequest(format!("无法解析日期: {v}")));
                }
                let mut info = DngDateTimeInfo::new();
                info.set_date_time(dt);
                doc.negative.metadata_mut().get_exif_mut().date_time_original = info;
                // XMP 端保留原始输入字符串（ISO 或 EXIF 格式均可）
                doc.negative.metadata_mut().get_xmp_mut().set_string(
                    XMP_NS_EXIF,
                    "exif:DateTimeOriginal",
                    &DngString::from_bytes(v.as_bytes()),
                );
                applied.push("date_time_original".into());
            }
            MetadataEdit::Rating(v) => {
                doc.negative
                    .metadata_mut()
                    .get_xmp_mut()
                    .set_int32(XMP_NS_XMP, "xmp:Rating", *v, false);
                applied.push(format!("rating={v}"));
            }
            MetadataEdit::Label(v) => {
                doc.negative.metadata_mut().get_xmp_mut().set_string(
                    XMP_NS_XMP,
                    "xmp:Label",
                    &DngString::from_bytes(v.as_bytes()),
                );
                applied.push("label".into());
            }
        }
    }

    if !applied.is_empty() {
        // 注意：这里**不调用** `synchronize_metadata()`。
        // 上游 dng-xmp 的 get_property 对数组节点返回 Some("")（dng-xmp/src/
        // node.rs:454），触发 sync_string 的「XMP 空值即删除」规则，会把
        // dc:creator 等数组属性清掉；且路径前缀语义与 C++ toolkit 不一致
        // （sync 用无前缀 "creator"，序列化认 "dc:creator"）。在上游修复前
        // （见 EVALUATION.md §2 摩擦点 6 / PROGRESS.md），本插件采用
        // 「双边显式写入」策略：EXIF 字段与 XMP 属性各写各的，跳过 sync。
        // IPTC 未被编辑时无需 Ingest；MWG 日期规则由编辑项自己保证。
    }

    Ok(MetadataEditReport {
        file: doc.path.display().to_string(),
        applied,
        after: after_values(doc),
    })
}

/// XMP 读取兜底：活对象写入用带前缀路径（"xmp:Rating"），而解析后的
/// 文件把节点存成无前缀名（"Rating"）——上游 dng-xmp 的路径语义在两个
/// 状态下不一致（见 EVALUATION.md）。读取时先试原路径，再试去掉前缀的
/// 本地名。公开给下游（测试/CLI/插件 ABI）共用。
pub fn xmp_read_string(xmp: &dng_sdk::dng_xmp::DngXmp, ns: &str, path: &str) -> Option<String> {
    let mut buf = DngString::new();
    if xmp.get_string(ns, path, &mut buf) && buf.not_empty() {
        return Some(buf.get_str().unwrap_or_default().to_string());
    }
    if let Some((_, local)) = path.rsplit_once(':') {
        let mut buf2 = DngString::new();
        if xmp.get_string(ns, local, &mut buf2) && buf2.not_empty() {
            return Some(buf2.get_str().unwrap_or_default().to_string());
        }
    }
    None
}

/// 编辑后从文档读回的实际值（negative 的 metadata 是工作副本，为权威来源；
/// `doc.info.exif` 只是解析时的文件快照，编辑不落在那里）。
fn after_values(doc: &DngDocument) -> AfterValues {
    let exif = doc.negative.metadata().get_exif();
    let xmp = doc.negative.metadata().get_xmp();

    let rating = xmp_read_string(xmp, XMP_NS_XMP, "xmp:Rating");
    let label = xmp_read_string(xmp, XMP_NS_XMP, "xmp:Label");

    AfterValues {
        image_description: exif.image_description.get_str().unwrap_or_default().to_string(),
        artist: exif.artist.get_str().unwrap_or_default().to_string(),
        copyright: exif.copyright.get_str().unwrap_or_default().to_string(),
        software: exif.software.get_str().unwrap_or_default().to_string(),
        user_comment: exif.user_comment.get_str().unwrap_or_default().to_string(),
        date_time_original: exif
            .date_time_original
            .is_valid()
            .then(|| exif.date_time_original.encode_iso_8601().get_str().unwrap_or_default().to_string()),
        rating,
        label,
    }
}

/// 另存选项。
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SaveOptions {
    /// true = 强制未压缩；false = 按像素类型自动（u16→无损 JPEG，f32→Deflate）
    pub uncompressed: bool,
    /// 向后兼容版本上限（None = dngVersion_SaveDefault）
    pub max_backward_version: Option<u32>,
}

impl Default for SaveOptions {
    fn default() -> Self {
        SaveOptions { uncompressed: false, max_backward_version: None }
    }
}

/// 另存结果报告。
#[derive(Debug, Serialize)]
pub struct SaveReport {
    pub output: String,
    pub file_size_bytes: u64,
    pub uncompressed: bool,
}

/// 把当前文档另存为 DNG。
///
/// 内部会先读 stage1 像素（写出管线需要）；由于缺口 E1（不嵌入
/// OriginalRawFileData），输出会重编码像素——这是与 Adobe DNG Converter
/// 的已知差异，报告中如实体现。
pub fn save_as_dng(
    doc: &mut DngDocument,
    out_path: impl AsRef<Path>,
    opts: &SaveOptions,
) -> DshResult<SaveReport> {
    doc.negative.read_stage1_image(&mut doc.host, &mut doc.stream, &mut doc.info)?;

    let mut out_stream = DngFileStream::open_write(out_path.as_ref())?;
    let mut writer = DngImageWriter::new();
    writer.write_dng(
        &mut doc.host,
        &mut out_stream,
        &mut doc.negative,
        opts.max_backward_version.unwrap_or(dngVersion_SaveDefault),
        opts.uncompressed,
        false,
    )?;
    drop(out_stream); // flush/关闭

    let size = std::fs::metadata(out_path.as_ref())?.len();
    Ok(SaveReport {
        output: out_path.as_ref().display().to_string(),
        file_size_bytes: size,
        uncompressed: opts.uncompressed,
    })
}

/// "YYYY-MM-DDTHH:MM:SS"（ISO）→ "YYYY:MM:DD HH:MM:SS"（EXIF）。
/// 已是 EXIF 格式则原样返回。
fn normalize_datetime(v: &str) -> DshResult<String> {
    let t = v.trim();
    if t.len() < 10 {
        return Err(DshError::BadRequest(format!("日期太短: {v}")));
    }
    if t.contains('-') || t.contains('T') {
        let normalized = t.replacen('-', ":", 1).replacen('-', ":", 1).replacen('T', " ", 1);
        Ok(normalized)
    } else {
        Ok(t.to_string())
    }
}
