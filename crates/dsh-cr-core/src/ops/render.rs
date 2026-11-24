//! 操作：渲染（对应 ACR 的「显影」——线性化/去马赛克/色彩变换后导出成品图）。
//!
//! 管线与 dng_validate 一致：read_stage1 → digest → stage2（线性化）→
//! stage3（去马赛克）→ `DngRender`（曝光/色调/色彩空间/位深）→
//! TIFF（write_tiff_basic）或 PPM。
//!
//! 色彩空间编号与 C++ dng_validate 的 -cs1..-cs8 对齐（sRGB/Adobe RGB/
//! ProPhoto/…），默认 sRGB 8-bit。

use serde::Serialize;

use dng_core::dng_color_space as cs;
use dng_core::dng_file_stream::DngFileStream;
use dng_core::dng_pixel_buffer::DngPixelBuffer;
use dng_core::dng_tag_types::{ttByte, ttFloat, ttShort};
use dng_core::dng_tag_values::{pcInterleaved, piBlackIsZero, piRGB};
use dng_sdk::dng_image_writer::{DngImageWriter, TiffWriteOptions};
use dng_sdk::dng_render::DngRender;

use crate::document::DngDocument;
use crate::error::{DshError, DshResult};

/// 渲染参数（JSON 请求体，插件 ABI 与 CLI 共用）。
#[derive(Debug, Clone, serde::Deserialize, Serialize)]
pub struct RenderParams {
    /// "srgb"(默认) | "adobe_rgb" | "pro_photo" | "gray18" | "gray22" |
    /// "display_p3" | "rec2020"，或 dng_validate 的编号 "1".."8"。
    pub color_space: Option<String>,
    /// "u8"(默认) | "u16" | "f32"。
    pub depth: Option<String>,
    /// 曝光增量（EV，加法作用于线性域；0 = 不动）。
    pub exposure: Option<f64>,
    /// 阴影调整（0 = 不动）。
    pub shadows: Option<f64>,
    /// 最长边上限（0/None = 不缩放）。
    pub max_size: Option<u32>,
}

impl Default for RenderParams {
    fn default() -> Self {
        RenderParams {
            color_space: None,
            depth: None,
            exposure: None,
            shadows: None,
            max_size: None,
        }
    }
}

/// 渲染结果报告。
#[derive(Debug, Serialize)]
pub struct RenderReport {
    pub file: String,
    pub output: String,
    /// "tiff" | "ppm"。
    pub format: String,
    pub width: u32,
    pub height: u32,
    pub planes: u32,
    /// 实际使用的色彩空间名。
    pub color_space: String,
    /// "u8" | "u16" | "f32"。
    pub depth: String,
    pub bytes: u64,
}

/// 渲染并导出为 TIFF（`.tif`）或 PPM（`.ppm`，8-bit）。
///
/// 输出格式由扩展名决定；PPM 强制 8-bit（PPM 标准限制）。
pub fn render_to(
    doc: &mut DngDocument,
    out_path: impl AsRef<std::path::Path>,
    params: &RenderParams,
) -> DshResult<RenderReport> {
    let out_path = out_path.as_ref();
    let ext = out_path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    let format = match ext.as_str() {
        "tif" | "tiff" => "tiff",
        "ppm" => "ppm",
        other => {
            return Err(DshError::BadRequest(format!(
                "不支持的渲染输出格式: .{other}（支持 .tif / .ppm）"
            )))
        }
    };

    // 像素管线（与 dng_validate 同序）：stage1 → digest → stage2 → stage3。
    doc.negative
        .read_stage1_image(&mut doc.host, &mut doc.stream, &mut doc.info)?;
    doc.negative.validate_raw_image_digest(&mut doc.host)?;
    doc.negative.build_stage2_image(&mut doc.host)?;
    doc.negative.build_stage3_image(&mut doc.host, -1)?;

    // 位深。
    let depth = params.depth.as_deref().unwrap_or("u8");
    let pixel_type = match (depth, format) {
        ("u16", "tiff") => ttShort,
        ("f32", "tiff") => ttFloat,
        ("u8", _) | (_, "ppm") => ttByte,
        (other, _) => {
            return Err(DshError::BadRequest(format!(
                "未知位深: {other}（支持 u8/u16/f32）"
            )))
        }
    };

    // 色彩空间（编号对齐 dng_validate -cs1..8）。
    let (space, space_name) = match params.color_space.as_deref().unwrap_or("srgb") {
        "srgb" | "1" => (cs::space_srgb(), "srgb"),
        "adobe_rgb" | "2" => (cs::space_adobe_rgb(), "adobe_rgb"),
        "pro_photo" | "3" => (cs::space_pro_photo(), "pro_photo"),
        "colormatch" | "4" => (cs::space_adobe_rgb(), "adobe_rgb(colormatch)"),
        "gray18" | "5" => (cs::space_gray_gamma18(), "gray18"),
        "gray22" | "6" => (cs::space_gray_gamma22(), "gray22"),
        "display_p3" | "7" => (cs::space_display_p3(), "display_p3"),
        "rec2020" | "8" => (cs::space_rec2020(), "rec2020"),
        other => {
            return Err(DshError::BadRequest(format!(
                "未知色彩空间: {other}"
            )))
        }
    };

    // 渲染。
    let mut render = DngRender::new(&doc.negative)?;
    render.set_final_pixel_type(pixel_type);
    render.set_final_space(
        space.matrix_from_pcs().clone(),
        space.gamma_function(),
        space.is_monochrome(),
    );
    if let Some(e) = params.exposure {
        render.set_exposure(e);
    }
    if let Some(s) = params.shadows {
        render.set_shadows(s);
    }
    if let Some(m) = params.max_size {
        if m > 0 {
            render.set_maximum_size(m);
        }
    }
    let rendered = render.render(&doc.negative)?;

    let img = rendered.read().unwrap();
    let (width, height, planes) = (img.width(), img.height(), img.planes());

    // 导出。
    let bytes = match format {
        "ppm" => write_ppm(out_path, &img)?,
        _ => {
            let photometric = if planes >= 3 { piRGB } else { piBlackIsZero };
            let mut out_stream = DngFileStream::open_write(out_path)?;
            let mut writer = DngImageWriter::new();
            writer.write_tiff_basic(
                &mut out_stream,
                &*img,
                &TiffWriteOptions {
                    photometric,
                    compression: dng_core::dng_tag_values::ccUncompressed,
                    icc_profile: None,
                    software: Some("dsh-camera-raw"),
                    metadata: None,
                },
            )?;
            drop(out_stream);
            std::fs::metadata(out_path)?.len()
        }
    };

    Ok(RenderReport {
        file: doc.path.display().to_string(),
        output: out_path.display().to_string(),
        format: format.to_string(),
        width,
        height,
        planes,
        color_space: space_name.to_string(),
        depth: if format == "ppm" { "u8".into() } else { depth.into() },
        bytes,
    })
}

/// 8-bit PPM 导出（与 dng_validate 的 write_ppm 逐字节同构，作为对照真值）。
fn write_ppm(
    path: &std::path::Path,
    image: &std::sync::RwLockReadGuard<dyn dng_core::DngImage>,
) -> DshResult<u64> {
    use std::io::Write;

    let width = image.width();
    let height = image.height();
    let planes = image.planes();

    let mut buf = DngPixelBuffer::with_planar_config(
        image.bounds(),
        0,
        planes,
        ttByte,
        pcInterleaved,
        DngPixelBuffer::allocate_data(image.bounds(), planes, ttByte)?,
    )?;

    image.do_get(&mut buf)?;

    let samples = buf
        .as_u8_slice()
        .ok_or_else(dng_core::program_error)?;

    let file = std::fs::File::create(path).map_err(dsh_error_io)?;
    let mut writer = std::io::BufWriter::new(file);

    if planes == 3 {
        write!(writer, "P6\n{width} {height}\n255\n").map_err(dsh_error_io)?;
    } else {
        write!(writer, "P5\n{width} {height}\n255\n").map_err(dsh_error_io)?;
    }
    writer.write_all(samples).map_err(dsh_error_io)?;
    writer.flush().map_err(dsh_error_io)?;

    Ok(std::fs::metadata(path)?.len())
}

fn dsh_error_io(e: std::io::Error) -> DshError {
    DshError::Io(e)
}
