//! DNG 文档模型：对应 ACR「打开一个文件」后的会话状态。
//!
//! 一次 `open` 完成 C++ SDK 中 `dng_info::Parse → dng_negative::Parse →
//! PostParse` 的完整元数据解析，之后所有操作共享这份状态。

use std::path::{Path, PathBuf};

use dng_core::{DngHost, dng_file_stream::DngFileStream};
use dng_sdk::dng_info::DngInfo;
use dng_sdk::dng_negative::DngNegative;

use crate::error::{DshError, DshResult};

/// 已打开的 DNG 文档。
///
/// 字段全部公开：操作层直接读写内部状态（与 C++ SDK 的使用方式一致）。
pub struct DngDocument {
    /// 打开的文件路径。
    pub path: PathBuf,
    /// 文件大小（字节）。
    pub file_size: u64,
    /// DNG Host——「宿主策略」入口（解析哪些阶段、线程数、预览策略等）。
    pub host: DngHost,
    /// 底层文件流，保持打开（后续读 stage1 像素、大表等仍需要）。
    pub stream: dng_core::dng_stream::DngStream<'static>,
    /// 解析后的 IFD 树 + 文件级元数据（EXIF 原始值、shared 块）。
    pub info: DngInfo,
    /// 解析后的 negative：相机模型、色彩、校正数据、opcode 列表等。
    pub negative: DngNegative,
}

impl DngDocument {
    /// 打开并解析 DNG（严格模式：`is_valid_dng` 不过则报 `NotDng`）。
    pub fn open(path: impl AsRef<Path>) -> DshResult<Self> {
        Self::open_inner(path, true)
    }

    /// 宽松模式：允许非法 DNG 通过打开阶段，供 validate 操作收集完整诊断。
    pub fn open_lenient(path: impl AsRef<Path>) -> DshResult<Self> {
        Self::open_inner(path, false)
    }

    fn open_inner(path: impl AsRef<Path>, require_valid: bool) -> DshResult<Self> {
        let path = path.as_ref().to_path_buf();
        let file_size = std::fs::metadata(&path).map_err(DshError::from)?.len();

        let mut host = DngHost::new();
        let mut stream = DngFileStream::open_read(&path)?;

        let mut info = DngInfo::new();
        info.parse(&mut host, &mut stream)?;
        info.post_parse(&mut host)?;

        let valid = info.is_valid_dng();
        if require_valid && !valid {
            return Err(DshError::NotDng(path.display().to_string()));
        }

        let mut negative = DngNegative::new(&mut host)?;
        negative.parse(&mut host, &mut stream, &mut info)?;
        negative.post_parse(&mut host, &mut stream, &mut info)?;

        Ok(DngDocument {
            path,
            file_size,
            host,
            stream,
            info,
            negative,
        })
    }
}
