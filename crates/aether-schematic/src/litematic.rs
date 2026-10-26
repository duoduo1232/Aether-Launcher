//! `.litematic`（Litematica「投影」文件）的读取与解码。
//!
//! 一个 litematic 文件是 gzip 压缩的 NBT：头部是名称、作者、MC 数据版本等
//! 元数据，`Regions` 下每个区域有自己的原点、尺寸、方块状态调色板和按
//! Litematica 自定义位打包（允许条目跨 long 边界）的 `BlockStates`。
//!
//! 边界与取舍：
//! - 解码服务于**预览**：单区域与总体积有上限（见 `MAX_REGION_VOLUME` /
//!   `MAX_TOTAL_VOLUME`），超过直接报错，而不是把内存吃满。
//! - 坐标尺寸取绝对值：Litematica 允许负尺寸表示区域朝负方向延伸，
//!   预览只关心形状。
//! - 方块索引的排列是 `(y·sizeZ + z)·sizeX + x`，与 Litematica 的写入顺序
//!   一致；预览端据此重建体素。
//! - 位宽 `bits = max(2, (palette_len - 1).bit_length())`，与 Litematica 的
//!   `LitematicaBitArray` 一致（条目连续排列、可跨越 long 边界）。经典打包
//!   覆盖了绝大多数现存文件；解码出与调色板不符的值或长度不符时报错，
//!   不产出静默错误的数据。

use serde::Serialize;
use thiserror::Error;

use crate::nbt::{NbtError, Value, read_gzip_nbt};

/// 单个区域允许的最大方块数（预览上限）。
pub const MAX_REGION_VOLUME: i64 = 8 * 1024 * 1024;

/// 整个投影文件允许的最大方块总数（预览上限）。
pub const MAX_TOTAL_VOLUME: i64 = 8 * 1024 * 1024;

/// 调色板条目上限（索引按 u16 编码）。
pub const MAX_PALETTE: usize = 65536;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SchematicError {
    #[error(transparent)]
    Nbt(#[from] NbtError),
    #[error("missing required litematic field {0}")]
    MissingField(&'static str),
    #[error("litematic field {0} has the wrong NBT type")]
    WrongFieldType(&'static str),
    #[error("region {0} volume {1} exceeds the preview limit of {MAX_REGION_VOLUME}")]
    RegionTooLarge(String, i64),
    #[error("total volume {0} exceeds the preview limit of {MAX_TOTAL_VOLUME}")]
    TotalTooLarge(i64),
    #[error("region palette exceeds the {MAX_PALETTE} entry limit")]
    PaletteTooLarge,
    #[error("region has an empty palette")]
    EmptyPalette,
    #[error("block state index {index} is outside the palette of {palette_len} entries")]
    InvalidPaletteIndex { index: u64, palette_len: usize },
    #[error("block states need {expected} longs for {volume} blocks, got {actual}")]
    BlockStatesLengthMismatch {
        expected: usize,
        actual: usize,
        volume: usize,
    },
    #[error("region {0} is missing its block states")]
    MissingBlockStates(String),
}

/// 投影文件头部的元数据。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LitematicMetadata {
    pub name: String,
    pub author: String,
    pub description: String,
    pub format_version: i32,
    pub minecraft_data_version: i32,
    pub region_count: i32,
    pub total_blocks: i64,
    pub total_volume: i64,
    pub enclosing_size: [i32; 3],
}

/// 一个区域的解码结果。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RegionData {
    pub name: String,
    pub position: [i32; 3],
    /// 取绝对值后的尺寸。
    pub size: [i32; 3],
    /// 方块状态字符串（如 `minecraft:stone`），按调色板 ID 排列。
    pub palette: Vec<String>,
    /// 调色板索引，按 `(y·size[2] + z)·size[0] + x` 排列。
    pub indices: Vec<u16>,
}

/// 解码完成的投影文件。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Schematic {
    pub metadata: LitematicMetadata,
    pub regions: Vec<RegionData>,
}

fn int_field(compound: &Value, name: &'static str) -> Result<i32, SchematicError> {
    compound
        .get(name)
        .and_then(Value::as_int)
        .ok_or(if compound.get(name).is_some() {
            SchematicError::WrongFieldType(name)
        } else {
            SchematicError::MissingField(name)
        })
}

fn long_field(compound: &Value, name: &'static str) -> Result<i64, SchematicError> {
    compound
        .get(name)
        .and_then(Value::as_long)
        .ok_or(if compound.get(name).is_some() {
            SchematicError::WrongFieldType(name)
        } else {
            SchematicError::MissingField(name)
        })
}

fn string_field(compound: &Value, name: &'static str) -> Result<String, SchematicError> {
    match compound.get(name) {
        Some(Value::String(text)) => Ok(text.clone()),
        Some(_) => Err(SchematicError::WrongFieldType(name)),
        None => Ok(String::new()),
    }
}

fn vec3_field(compound: &Value, name: &'static str) -> Result<[i32; 3], SchematicError> {
    let nested = compound
        .get(name)
        .ok_or(SchematicError::MissingField(name))?;
    Ok([
        int_field(nested, "x")?,
        int_field(nested, "y")?,
        int_field(nested, "z")?,
    ])
}

fn metadata_from_root(root: &Value) -> Result<LitematicMetadata, SchematicError> {
    Ok(LitematicMetadata {
        name: string_field(root, "Name")?,
        author: string_field(root, "Author")?,
        description: string_field(root, "Description")?,
        format_version: int_field(root, "Version")?,
        minecraft_data_version: int_field(root, "MinecraftDataVersion")?,
        region_count: int_field(root, "RegionCount")?,
        total_blocks: long_field(root, "TotalBlocks")?,
        total_volume: long_field(root, "TotalVolume")?,
        enclosing_size: vec3_field(root, "EnclosingSize")?,
    })
}

/// 只读取元数据，不解码任何 `BlockStates`。
///
/// # Errors
/// 文件不是合法的 gzip NBT 或缺少必需的 litematic 字段时返回
/// [`SchematicError`]。
pub fn read_metadata(bytes: &[u8]) -> Result<LitematicMetadata, SchematicError> {
    let (_, root) = read_gzip_nbt(bytes)?;
    metadata_from_root(&root)
}

/// 解码一个区域的 `BlockStates`：Litematica 位打包 → 调色板索引序列。
///
/// `volume` 是取绝对值后的方块总数；返回向量按 `(y·sizeZ + z)·sizeX + x`
/// 排列，与写入顺序一致。
///
/// # Errors
/// 调色板为空或超限、long 数组长度与位宽不匹配、解码出的索引越出调色板时
/// 返回 [`SchematicError`]。
pub fn decode_region_blocks(
    palette_len: usize,
    longs: &[i64],
    volume: usize,
) -> Result<Vec<u16>, SchematicError> {
    if palette_len == 0 {
        return Err(SchematicError::EmptyPalette);
    }
    if palette_len > MAX_PALETTE {
        return Err(SchematicError::PaletteTooLarge);
    }
    if volume == 0 {
        return Ok(Vec::new());
    }
    let bits = bits_per_entry(palette_len);
    let expected = volume
        .checked_mul(bits)
        .map(|total_bits| total_bits.div_ceil(64))
        .ok_or(SchematicError::RegionTooLarge(
            "volume".to_owned(),
            volume as i64,
        ))?;
    if longs.len() != expected {
        return Err(SchematicError::BlockStatesLengthMismatch {
            expected,
            actual: longs.len(),
            volume,
        });
    }

    let mask = if bits >= 64 {
        u64::MAX
    } else {
        (1u64 << bits) - 1
    };
    let mut indices = Vec::with_capacity(volume);
    for index in 0..volume {
        let start = index * bits;
        let long_index = start >> 6;
        let offset = start & 63;
        let mut value = (longs[long_index] as u64) >> offset;
        if offset + bits > 64 {
            value |= (longs[long_index + 1] as u64) << (64 - offset);
        }
        value &= mask;
        let value = value as usize;
        if value >= palette_len {
            return Err(SchematicError::InvalidPaletteIndex {
                index: value as u64,
                palette_len,
            });
        }
        indices.push(value as u16);
    }
    Ok(indices)
}

/// Litematica 的位宽：至少 2 位，否则为容纳最大调色板索引所需的位数
/// （等价于 `(palette_len - 1)` 的位长度）。
#[must_use]
pub fn bits_per_entry(palette_len: usize) -> usize {
    if palette_len <= 2 {
        2
    } else {
        let max_index = palette_len - 1;
        (usize::BITS - max_index.leading_zeros()) as usize
    }
}

/// 解析并解码完整的投影文件。
///
/// # Errors
/// 见 [`SchematicError`]；单区域或总体积超过预览上限时也会报错。
pub fn parse(bytes: &[u8]) -> Result<Schematic, SchematicError> {
    let (_, root) = read_gzip_nbt(bytes)?;
    let metadata = metadata_from_root(&root)?;
    if metadata.total_volume > MAX_TOTAL_VOLUME {
        return Err(SchematicError::TotalTooLarge(metadata.total_volume));
    }

    let regions_value = root
        .get("Regions")
        .ok_or(SchematicError::MissingField("Regions"))?;
    let mut regions = Vec::new();
    let mut decoded_total: i64 = 0;
    if let Value::Compound(entries) = regions_value {
        for (name, region) in entries {
            let position = vec3_field(region, "Position")?;
            let raw_size = vec3_field(region, "Size")?;
            let size = [raw_size[0].abs(), raw_size[1].abs(), raw_size[2].abs()];
            let volume = i64::from(size[0]) * i64::from(size[1]) * i64::from(size[2]);
            if volume > MAX_REGION_VOLUME {
                return Err(SchematicError::RegionTooLarge(name.clone(), volume));
            }
            decoded_total += volume;
            if decoded_total > MAX_TOTAL_VOLUME {
                return Err(SchematicError::TotalTooLarge(decoded_total));
            }

            let palette_value = region
                .get("Palette")
                .ok_or(SchematicError::MissingField("Palette"))?;
            let palette_entries = palette_value
                .as_compound()
                .ok_or(SchematicError::WrongFieldType("Palette"))?;
            let mut palette = vec![String::new(); palette_entries.len()];
            for (block_state, id) in palette_entries {
                let id = id
                    .as_int()
                    .ok_or(SchematicError::WrongFieldType("Palette"))?;
                let slot = usize::try_from(id).map_err(|_| SchematicError::PaletteTooLarge)?;
                if slot >= palette.len() {
                    return Err(SchematicError::PaletteTooLarge);
                }
                palette[slot] = block_state.clone();
            }

            let longs = region
                .get("BlockStates")
                .and_then(Value::as_long_array)
                .ok_or_else(|| SchematicError::MissingBlockStates(name.clone()))?;
            let indices = decode_region_blocks(palette.len(), longs, volume as usize)?;

            regions.push(RegionData {
                name: name.clone(),
                position,
                size,
                palette,
                indices,
            });
        }
    } else {
        return Err(SchematicError::WrongFieldType("Regions"));
    }

    Ok(Schematic { metadata, regions })
}

#[cfg(test)]
#[path = "litematic_tests.rs"]
mod tests;
