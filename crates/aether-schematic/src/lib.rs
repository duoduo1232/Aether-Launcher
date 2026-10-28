//! `.litematic`（Litematica「投影」）文件的读取与解码。
//!
//! 面向启动器的投影图预览与管理：从受管理的目录读取投影文件，解出元数据
//! 与逐区域的方块索引，供前端做 3D 渲染与跨版本信息展示。解析在受信任的
//! Rust 层完成并施加大小上限，前端只拿渲染数据，不接触文件路径。

pub mod data_versions;
pub mod litematic;
pub mod nbt;

pub use litematic::{
    LitematicMetadata, MAX_PALETTE, MAX_REGION_VOLUME, MAX_TOTAL_VOLUME, RegionData, Schematic,
    SchematicError, bits_per_entry, decode_region_blocks, parse, read_metadata, read_metadata_from,
};
pub use nbt::{NbtError, Value};
