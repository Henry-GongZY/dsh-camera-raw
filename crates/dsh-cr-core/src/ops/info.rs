//! 操作：读取 DNG 文件信息（对应 ACR 打开文件后的 Header / Info 面板，
//! 以及 Adobe DNG Converter 的信息展示）。

use serde::Serialize;

use dng_core::dng_tag_values::{
    ccDeflate, ccJXL, ccJPEG, ccLZW, ccLossyJPEG, ccOldJPEG, ccPackBits, ccUncompressed,
    piBlackIsZero, piCFA, piDepth, piLinearRaw, piRGB, piTransparencyMask, piWhiteIsZero, piYCbCr,
};
use dng_core::dng_tag_types::{ttByte, ttFloat, ttHalfFloat, ttLong, ttShort};

use crate::document::DngDocument;

/// DNG 文件信息汇总（JSON 报告）。
#[derive(Debug, Serialize)]
pub struct DngSummary {
    pub file: String,
    pub file_size_bytes: u64,
    /// "little" / "big"。
    pub byte_order: String,
    /// 形如 "1.4.0.0"。
    pub dng_version: String,
    pub dng_backward_version: String,
    pub is_valid_dng: bool,
    pub camera: CameraSummary,
    /// 主 RAW IFD（SubIFD / 主 IFD 中的最大分辨率 RAW 图）摘要。
    pub raw_ifd: Option<RawIfdSummary>,
    pub ifd_count: u32,
    pub chained_ifd_count: u32,
    pub indices: IndicesSummary,
    pub color: ColorSummary,
    pub opcodes: Vec<OpcodeListSummary>,
    pub exif: ExifSummary,
}

#[derive(Debug, Serialize)]
pub struct CameraSummary {
    pub make: String,
    pub model: String,
    pub unique_camera_model: String,
    pub localized_camera_model: String,
}

#[derive(Debug, Serialize)]
pub struct RawIfdSummary {
    /// 在 `info.ifd` 中的下标。
    pub index: i32,
    pub width: u32,
    pub height: u32,
    pub bits_per_sample: Vec<u32>,
    pub sample_format: Vec<String>,
    pub pixel_type: String,
    pub compression: String,
    pub photometric: String,
    pub cfa: Option<CfaSummary>,
    pub tiling: TilingSummary,
    pub new_sub_file_type: u32,
}

#[derive(Debug, Serialize)]
pub struct CfaSummary {
    /// [rows, cols]。
    pub repeat_pattern: [u32; 2],
    /// 每行的颜色字母序列，如 ["RG", "GB"]。
    pub rows: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct TilingSummary {
    pub uses_tiles: bool,
    pub tile_width: u32,
    pub tile_length: u32,
    pub tiles_across: u32,
    pub tiles_down: u32,
    pub uses_strips: bool,
}

#[derive(Debug, Serialize)]
pub struct IndicesSummary {
    pub main_index: i32,
    pub mask_index: i32,
    pub depth_index: i32,
    pub enhanced_index: i32,
    pub semantic_mask_indices: Vec<u32>,
}

#[derive(Debug, Serialize)]
pub struct ColorSummary {
    pub color_channels: u32,
    pub is_monochrome: bool,
    pub as_shot_neutral: Option<Vec<f64>>,
    /// 有理数原样字符串，如 "0/1"。
    pub baseline_exposure: String,
    pub profiles: Vec<ProfileSummary>,
}

#[derive(Debug, Serialize)]
pub struct ProfileSummary {
    pub name: String,
    pub is_embedded: bool,
}

#[derive(Debug, Serialize)]
pub struct OpcodeListSummary {
    /// 1 / 2 / 3。
    pub list: u32,
    pub count: u32,
    pub opcode_ids: Vec<u32>,
}

#[derive(Debug, Serialize)]
pub struct ExifSummary {
    pub image_description: String,
    pub make: String,
    pub model: String,
    pub software: String,
    pub artist: String,
    pub copyright: String,
    pub date_time: Option<String>,
    pub date_time_original: Option<String>,
    pub exposure_time: Option<f64>,
    pub f_number: Option<f64>,
    pub iso: Option<u32>,
    pub focal_length_mm: Option<f64>,
    pub focal_length_35mm: Option<u32>,
}

/// 生成 DNG 文件信息汇总。
pub fn summarize(doc: &mut DngDocument) -> DngSummary {
    let shared = &doc.info.shared;
    // 用 negative 的 metadata 工作副本（编辑会落在那里），而非解析快照
    let exif = doc.negative.metadata().get_exif();

    let camera = CameraSummary {
        make: dng_string(&exif.make),
        model: dng_string(&exif.model),
        unique_camera_model: dng_string(&shared.unique_camera_model),
        localized_camera_model: dng_string(&shared.localized_camera_model),
    };

    let raw_ifd = (doc.info.main_index >= 0).then(|| {
        let idx = doc.info.main_index;
        let ifd = &doc.info.ifd[idx as usize];
        RawIfdSummary {
            index: idx,
            width: ifd.image_width,
            height: ifd.image_length,
            bits_per_sample: ifd.bits_per_sample[..ifd.samples_per_pixel.min(4) as usize].to_vec(),
            sample_format: ifd.sample_format[..ifd.samples_per_pixel.min(4) as usize]
                .iter()
                .map(|&v| sample_format_name(v))
                .collect(),
            pixel_type: pixel_type_name(ifd.pixel_type()),
            compression: compression_name(ifd.compression).to_string(),
            photometric: photometric_name(ifd.photometric_interpretation).to_string(),
            cfa: cfa_summary(ifd),
            tiling: TilingSummary {
                uses_tiles: ifd.uses_tiles,
                tile_width: ifd.tile_width,
                tile_length: ifd.tile_length,
                tiles_across: ifd.tiles_across(),
                tiles_down: ifd.tiles_down(),
                uses_strips: ifd.uses_strips,
            },
            new_sub_file_type: ifd.new_sub_file_type,
        }
    });

    let as_shot_neutral = shared.as_shot_neutral.as_ref().map(|v| {
        (0..v.count())
            .map(|i| v.get(i as usize))
            .collect::<Vec<f64>>()
    });

    let profiles = (0..doc.negative.profile_count())
        .map(|i| {
            let p = doc.negative.profile_by_index(i);
            ProfileSummary {
                name: dng_string(p.name()),
                is_embedded: p.name_is_embedded(),
            }
        })
        .collect();

    let color = ColorSummary {
        color_channels: doc.negative.color_channels(),
        is_monochrome: doc.negative.is_monochrome(),
        as_shot_neutral,
        baseline_exposure: format!("{}/{}", shared.baseline_exposure.n, shared.baseline_exposure.d),
        profiles,
    };

    let opcodes = [1u32, 2, 3]
        .iter()
        .map(|&n| {
            let list = match n {
                1 => doc.negative.opcode_list1(),
                2 => doc.negative.opcode_list2(),
                _ => doc.negative.opcode_list3(),
            };
            OpcodeListSummary {
                list: n,
                count: list.count(),
                opcode_ids: (0..list.count())
                    .map(|i| list.entry(i).opcode_id())
                    .collect(),
            }
        })
        .filter(|l| l.count > 0)
        .collect();

    let exif_summary = ExifSummary {
        image_description: dng_string(&exif.image_description),
        make: dng_string(&exif.make),
        model: dng_string(&exif.model),
        software: dng_string(&exif.software),
        artist: dng_string(&exif.artist),
        copyright: dng_string(&exif.copyright),
        date_time: iso8601_if_valid(&exif.date_time),
        date_time_original: iso8601_if_valid(&exif.date_time_original),
        exposure_time: urational_f64(&exif.exposure_time),
        f_number: urational_f64(&exif.fnumber),
        iso: exif.iso_speed_ratings.iter().find(|&&v| v != 0).copied(),
        focal_length_mm: urational_f64(&exif.focal_length),
        focal_length_35mm: (exif.focal_length_in_35mm_film != 0)
            .then_some(exif.focal_length_in_35mm_film),
    };

    DngSummary {
        file: doc.path.display().to_string(),
        file_size_bytes: doc.file_size,
        byte_order: if doc.info.big_endian { "big" } else { "little" }.into(),
        dng_version: format_version(shared.dng_version),
        dng_backward_version: format_version(shared.dng_backward_version),
        is_valid_dng: doc.info.is_valid_dng(),
        camera,
        raw_ifd,
        ifd_count: doc.info.ifd_count(),
        chained_ifd_count: doc.info.chained_ifd_count(),
        indices: IndicesSummary {
            main_index: doc.info.main_index,
            mask_index: doc.info.mask_index,
            depth_index: doc.info.depth_index,
            enhanced_index: doc.info.enhanced_index,
            semantic_mask_indices: doc.info.semantic_mask_indices.clone(),
        },
        color,
        opcodes,
        exif: exif_summary,
    }
}

// ---------------------------------------------------------------- helpers

/// `DngString` -> `String`（RustDNG 的 DngString 自带 UTF-8 修复语义，非 String 别名）。
fn dng_string(s: &dng_core::DngString) -> String {
    s.get_str().unwrap_or_default().to_string()
}

fn iso8601_if_valid(dt: &dng_core::dng_date_time::DngDateTimeInfo) -> Option<String> {
    dt.is_valid().then(|| dng_string(&dt.encode_iso_8601()))
}

fn urational_f64(r: &dng_core::DngURational) -> Option<f64> {
    (r.d != 0).then(|| r.n as f64 / r.d as f64)
}

fn format_version(v: u32) -> String {
    format_dng_version(v)
}

/// DNG 版本号（u32 打包值）→ "1.4.0.0" 字符串。
pub fn format_dng_version(v: u32) -> String {
    format!("{}.{}.{}.{}", (v >> 24) & 0xff, (v >> 16) & 0xff, (v >> 8) & 0xff, v & 0xff)
}

fn compression_name(c: u32) -> &'static str {
    match c {
        ccUncompressed => "Uncompressed",
        ccLZW => "LZW",
        ccOldJPEG => "OldJPEG",
        ccJPEG => "JPEG",
        ccDeflate => "Deflate",
        ccPackBits => "PackBits",
        // 注意：RustDNG 沿用 C++ 命名 ccLossyJPEG = 34892（DNG 规范中的 Lossless JPEG）
        ccLossyJPEG => "LosslessJPEG",
        ccJXL => "JXL",
        _ => "Unknown",
    }
}

fn photometric_name(p: u32) -> &'static str {
    match p {
        piWhiteIsZero => "WhiteIsZero",
        piBlackIsZero => "BlackIsZero",
        piRGB => "RGB",
        piTransparencyMask => "TransparencyMask",
        piYCbCr => "YCbCr",
        piCFA => "CFA",
        piLinearRaw => "LinearRaw",
        piDepth => "Depth",
        _ => "Unknown",
    }
}

fn pixel_type_name(t: u32) -> String {
    match t {
        ttByte => "Byte".into(),
        ttShort => "Short".into(),
        ttLong => "Long".into(),
        ttFloat => "Float32".into(),
        ttHalfFloat => "Float16".into(),
        _ => format!("Type({t})"),
    }
}

fn sample_format_name(v: u32) -> String {
    match v {
        1 => "Unsigned".into(),
        2 => "Signed".into(),
        3 => "Float".into(),
        4 => "Undefined".into(),
        _ => format!("Unknown({v})"),
    }
}

fn cfa_summary(ifd: &dng_core::dng_ifd::DngIfd) -> Option<CfaSummary> {
    if ifd.cfa_repeat_pattern_rows == 0 || ifd.cfa_repeat_pattern_cols == 0 {
        return None;
    }
    // CFAPlaneColor 代码 → 字母：0=R, 1=G, 2=B（DNG 默认面顺序）
    let code_letter = |c: u8| match c {
        0 => 'R',
        1 => 'G',
        2 => 'B',
        _ => '?',
    };
    let rows = (0..ifd.cfa_repeat_pattern_rows as usize)
        .map(|r| {
            (0..ifd.cfa_repeat_pattern_cols as usize)
                .map(|c| code_letter(ifd.cfa_pattern[r][c]))
                .collect::<String>()
        })
        .collect();
    Some(CfaSummary {
        repeat_pattern: [ifd.cfa_repeat_pattern_rows, ifd.cfa_repeat_pattern_cols],
        rows,
    })
}
