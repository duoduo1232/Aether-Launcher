use std::io::Write;

use super::{
    MAX_REGION_VOLUME, SchematicError, bits_per_entry, decode_region_blocks, parse, read_metadata,
};

// ---------------------------------------------------------------------------
// 测试专用：NBT 写入器（只覆盖 litematic 需要的类型）
// ---------------------------------------------------------------------------

struct RootBuilder {
    out: Vec<u8>,
}

impl RootBuilder {
    fn new() -> Self {
        let mut out = vec![10u8];
        out.extend_from_slice(&0u16.to_be_bytes()); // 根复合名为空
        Self { out }
    }

    fn int(&mut self, name: &str, value: i32) {
        self.tag_payload(3, name, &value.to_be_bytes());
    }

    fn long(&mut self, name: &str, value: i64) {
        self.tag_payload(4, name, &value.to_be_bytes());
    }

    fn string(&mut self, name: &str, value: &str) {
        self.tag_payload(8, name, &{
            let mut payload = (value.len() as u16).to_be_bytes().to_vec();
            payload.extend_from_slice(value.as_bytes());
            payload
        });
    }

    fn begin_compound(&mut self, name: &str) {
        self.tag_payload(10, name, &[]);
    }

    fn long_array(&mut self, name: &str, values: &[i64]) {
        self.tag_payload(12, name, &{
            let mut payload = (values.len() as i32).to_be_bytes().to_vec();
            for value in values {
                payload.extend_from_slice(&value.to_be_bytes());
            }
            payload
        });
    }

    /// 列表头：元素类型 + 长度。元素本身只写负载——没有类型字节也没有名字，
    /// 复合元素直接写字段再用 `end_compound()` 收尾。
    fn begin_list(&mut self, name: &str, element_tag: u8, len: usize) {
        self.out.push(9);
        self.out
            .extend_from_slice(&(name.len() as u16).to_be_bytes());
        self.out.extend_from_slice(name.as_bytes());
        self.out.push(element_tag);
        self.out.extend_from_slice(&(len as i32).to_be_bytes());
    }

    fn end_compound(&mut self) {
        self.out.push(0);
    }

    fn tag_payload(&mut self, tag: u8, name: &str, payload: &[u8]) {
        self.out.push(tag);
        self.out
            .extend_from_slice(&(name.len() as u16).to_be_bytes());
        self.out.extend_from_slice(name.as_bytes());
        self.out.extend_from_slice(payload);
    }

    fn gzip(mut self) -> Vec<u8> {
        self.out.push(0); // 根复合结束
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&self.out).expect("gzip");
        encoder.finish().expect("finish")
    }
}

// ---------------------------------------------------------------------------
// Litematica 位打包的测试编码器（与解码器互为镜像）
// ---------------------------------------------------------------------------

fn encode_litematica_bits(values: &[u64], bits: usize) -> Vec<i64> {
    let total_bits = values.len() * bits;
    let long_count = total_bits.div_ceil(64);
    let mut longs = vec![0i64; long_count];
    for (index, &value) in values.iter().enumerate() {
        let start = index * bits;
        let long_index = start >> 6;
        let offset = start & 63;
        longs[long_index] |= ((value as i64) & ((1i64 << bits) - 1)) << offset;
        if offset + bits > 64 {
            let spill = offset + bits - 64;
            longs[long_index + 1] |= ((value as i64) & ((1i64 << bits) - 1)) >> (bits - spill);
        }
    }
    longs
}

// ---------------------------------------------------------------------------
// 位宽与解码
// ---------------------------------------------------------------------------

#[test]
fn bits_per_entry_follows_litematica_widths() {
    assert_eq!(bits_per_entry(1), 2);
    assert_eq!(bits_per_entry(2), 2);
    assert_eq!(bits_per_entry(3), 2);
    assert_eq!(bits_per_entry(4), 2);
    assert_eq!(bits_per_entry(5), 3);
    assert_eq!(bits_per_entry(8), 3);
    assert_eq!(bits_per_entry(9), 4);
    assert_eq!(bits_per_entry(17), 5);
}

#[test]
fn decodes_a_known_bit_pattern() {
    // 调色板 5 项 → 3 位；[0,1,2,3,4] 打包进一个 long：
    // 0 | 1<<3 | 2<<6 | 3<<9 | 4<<12 = 18056。
    assert_eq!(
        decode_region_blocks(5, &[18_056], 5).expect("decode"),
        vec![0, 1, 2, 3, 4]
    );
}

#[test]
fn decode_crosses_long_boundaries() {
    // 3 位 × 30 项一定跨 long 边界；用编码器构造再解码，要求逐值一致。
    let values: Vec<u64> = (0..30).map(|index| (index * 7 + 3) % 5).collect();
    let longs = encode_litematica_bits(&values, 3);
    let expected: Vec<u16> = values.iter().map(|&value| value as u16).collect();
    assert_eq!(
        decode_region_blocks(5, &longs, 30).expect("decode"),
        expected
    );
}

#[test]
fn decode_rejects_wrong_length_and_out_of_range_index() {
    // 5 项调色板 3 位，8 个方块需要 ceil(24/64)=1 个 long。
    assert!(matches!(
        decode_region_blocks(5, &[], 8),
        Err(SchematicError::BlockStatesLengthMismatch {
            expected: 1,
            actual: 0,
            volume: 8
        })
    ));
    // 值 5 越出 5 项调色板：打包 [5] 进 3 位。
    assert!(matches!(
        decode_region_blocks(5, &[5], 1),
        Err(SchematicError::InvalidPaletteIndex {
            index: 5,
            palette_len: 5
        })
    ));
    assert_eq!(
        decode_region_blocks(0, &[], 0),
        Err(SchematicError::EmptyPalette)
    );
}

// ---------------------------------------------------------------------------
// 完整文件解析
// ---------------------------------------------------------------------------

/// 构造一个 2×2×2、两色调色板（空气/石头）的最小投影文件。
///
/// 布局与 Litematica 真正写出的文件一致：`Metadata` 是根下的复合，
/// `TotalBlocks` / `TotalVolume` 是 `TAG_Int`，区域调色板是
/// `BlockStatePalette` 列表（元素为带 `Name` 的复合）。
fn minimal_litematic(total_volume: i32, region_size: (i32, i32, i32)) -> Vec<u8> {
    // 2 项调色板 → 2 位；8 个方块 = 16 位 = 1 个 long 的一半。
    let blocks: Vec<u64> = vec![1, 0, 1, 1, 0, 0, 1, 1];
    let longs = encode_litematica_bits(&blocks, 2);

    let mut root = RootBuilder::new();
    root.int("Version", 6);
    root.int("SubVersion", 1);
    root.int("MinecraftDataVersion", 3955);
    root.begin_compound("Metadata");
    root.string("Name", "测试投影");
    root.string("Author", "tester");
    root.string("Description", "fixture");
    root.int("RegionCount", 1);
    root.int("TotalBlocks", 5);
    root.int("TotalVolume", total_volume);
    root.begin_compound("EnclosingSize");
    root.int("x", region_size.0);
    root.int("y", region_size.1);
    root.int("z", region_size.2);
    root.end_compound();
    root.end_compound(); // Metadata
    root.begin_compound("Regions");
    root.begin_compound("main");
    root.begin_compound("Position");
    root.int("x", 0);
    root.int("y", 0);
    root.int("z", 0);
    root.end_compound();
    root.begin_compound("Size");
    root.int("x", region_size.0);
    root.int("y", region_size.1);
    root.int("z", region_size.2);
    root.end_compound();
    // 列表元素只有负载：没有类型字节，也没有名字，复合元素以 0 结束。
    root.begin_list("BlockStatePalette", 10, 2);
    root.string("Name", "minecraft:air");
    root.end_compound();
    root.string("Name", "minecraft:stone");
    root.begin_compound("Properties");
    root.string("variant", "smooth");
    root.end_compound();
    root.end_compound();
    root.long_array("BlockStates", &longs);
    root.end_compound(); // main
    root.end_compound(); // Regions
    root.gzip()
}

#[test]
fn parses_metadata_and_region_blocks() {
    let bytes = minimal_litematic(8, (2, 2, 2));

    let metadata = read_metadata(&bytes).expect("metadata");
    assert_eq!(metadata.name, "测试投影");
    assert_eq!(metadata.author, "tester");
    assert_eq!(metadata.format_version, 6);
    assert_eq!(metadata.minecraft_data_version, 3955);
    assert_eq!(metadata.total_blocks, 5);
    assert_eq!(metadata.total_volume, 8);
    assert_eq!(metadata.enclosing_size, [2, 2, 2]);

    let schematic = parse(&bytes).expect("parse");
    assert_eq!(schematic.regions.len(), 1);
    let region = &schematic.regions[0];
    assert_eq!(region.name, "main");
    assert_eq!(region.size, [2, 2, 2]);
    assert_eq!(
        region.palette,
        vec!["minecraft:air".to_owned(), "minecraft:stone".to_owned()]
    );
    assert_eq!(region.indices, vec![1, 0, 1, 1, 0, 0, 1, 1]);
}

#[test]
fn long_counts_are_accepted_too() {
    // 计数语义上不限定整型宽度：`TAG_Int` 和 `TAG_Long` 都要认。
    let mut root = RootBuilder::new();
    root.int("Version", 6);
    root.int("MinecraftDataVersion", 3955);
    root.begin_compound("Metadata");
    root.int("RegionCount", 0);
    root.long("TotalBlocks", 7);
    root.long("TotalVolume", 9);
    root.begin_compound("EnclosingSize");
    root.int("x", 1);
    root.int("y", 1);
    root.int("z", 1);
    root.end_compound();
    root.end_compound(); // Metadata

    let metadata = read_metadata(&root.gzip()).expect("metadata");
    assert_eq!(metadata.total_blocks, 7);
    assert_eq!(metadata.total_volume, 9);
}

#[test]
fn metadata_read_does_not_require_block_states() {
    // 构造一个 BlockStates 长度错误的文件：读元数据应成功，完整解析应失败。
    let mut root = RootBuilder::new();
    root.int("Version", 6);
    root.int("MinecraftDataVersion", 3955);
    root.begin_compound("Metadata");
    root.int("RegionCount", 1);
    root.int("TotalBlocks", 0);
    root.int("TotalVolume", 8);
    root.begin_compound("EnclosingSize");
    root.int("x", 2);
    root.int("y", 2);
    root.int("z", 2);
    root.end_compound();
    root.end_compound(); // Metadata
    root.begin_compound("Regions");
    root.begin_compound("main");
    root.begin_compound("Position");
    root.int("x", 0);
    root.int("y", 0);
    root.int("z", 0);
    root.end_compound();
    root.begin_compound("Size");
    root.int("x", 2);
    root.int("y", 2);
    root.int("z", 2);
    root.end_compound();
    root.begin_list("BlockStatePalette", 10, 1);
    root.string("Name", "minecraft:air");
    root.string("Name", "minecraft:air");
    root.end_compound();
    root.long_array("BlockStates", &[]); // 需要 1 个 long，给了 0 个
    root.end_compound(); // main
    root.end_compound(); // Regions

    let gzip = root.gzip();
    assert!(read_metadata(&gzip).is_ok());
    assert!(matches!(
        parse(&gzip),
        Err(SchematicError::BlockStatesLengthMismatch { .. })
    ));
}

#[test]
fn negative_region_size_uses_absolute_value() {
    let bytes = minimal_litematic(8, (-2, 2, -2));
    let schematic = parse(&bytes).expect("parse");
    assert_eq!(schematic.regions[0].size, [2, 2, 2]);
}

#[test]
fn oversize_total_volume_is_rejected_before_decoding() {
    let bytes = minimal_litematic((MAX_REGION_VOLUME + 1) as i32, (2, 2, 2));
    assert!(matches!(
        parse(&bytes),
        Err(SchematicError::TotalTooLarge(volume)) if volume == MAX_REGION_VOLUME + 1
    ));
}

#[test]
fn missing_metadata_is_reported() {
    let mut root = RootBuilder::new();
    root.int("Version", 6);
    root.int("MinecraftDataVersion", 3955);
    let gzip = root.gzip();
    assert!(matches!(
        read_metadata(&gzip),
        Err(SchematicError::MissingField("Metadata"))
    ));
}

#[test]
fn missing_required_field_is_reported() {
    let mut root = RootBuilder::new();
    root.int("Version", 6);
    // 缺 MinecraftDataVersion。
    root.begin_compound("Metadata");
    root.int("RegionCount", 0);
    root.int("TotalBlocks", 0);
    root.int("TotalVolume", 0);
    root.begin_compound("EnclosingSize");
    root.int("x", 0);
    root.int("y", 0);
    root.int("z", 0);
    root.end_compound();
    root.end_compound(); // Metadata
    let gzip = root.gzip();
    assert!(matches!(
        read_metadata(&gzip),
        Err(SchematicError::MissingField("MinecraftDataVersion"))
    ));
}

// ---------------------------------------------------------------------------
// 数据版本对照表
// ---------------------------------------------------------------------------

#[test]
fn nearest_release_maps_data_versions() {
    use crate::data_versions::nearest_release;
    assert_eq!(nearest_release(1343), Some("1.12.2"));
    assert_eq!(nearest_release(1342), None);
    assert_eq!(nearest_release(3954), Some("1.21"));
    assert_eq!(nearest_release(4189), Some("1.21.4"));
    assert_eq!(nearest_release(999_999), Some("1.21.8"));
}

#[test]
fn metadata_is_read_even_when_regions_comes_first() {
    // Litematica 的字段顺序不固定：`Regions` 可能排在 `Metadata` 前面。
    // 提前停止解压时不能因此漏掉 `Version` 这些根上的小字段。
    let mut root = RootBuilder::new();
    root.begin_compound("Regions");
    root.begin_compound("castle");
    root.begin_compound("Size");
    root.int("x", 1);
    root.int("y", 1);
    root.int("z", 1);
    root.end_compound();
    root.end_compound(); // castle
    root.end_compound(); // Regions
    root.int("Version", 6);
    root.int("MinecraftDataVersion", 3955);
    root.begin_compound("Metadata");
    root.int("RegionCount", 1);
    root.int("TotalBlocks", 1);
    root.int("TotalVolume", 1);
    root.begin_compound("EnclosingSize");
    root.int("x", 1);
    root.int("y", 1);
    root.int("z", 1);
    root.end_compound();
    root.end_compound(); // Metadata

    let metadata = read_metadata(&root.gzip()).expect("metadata");
    assert_eq!(metadata.format_version, 6);
    assert_eq!(metadata.minecraft_data_version, 3955);
    assert_eq!(metadata.region_count, 1);
}

/// 负 `Size` 的选区：起点要按 `min(Position, Position + Size - 1)` 归一化。
///
/// Litematica 允许反向选区，这时 `Position` 是最大的那个角，真正的原点在
/// `Position + Size - 1`。直接用 `Position` 会让整块建筑平移一个尺寸，
/// 表现为「位置错的」。
#[test]
fn a_negative_size_region_starts_at_the_min_corner() {
    let block: Vec<u64> = vec![1];
    let longs = encode_litematica_bits(&block, 1);

    let mut root = RootBuilder::new();
    root.int("Version", 6);
    root.int("MinecraftDataVersion", 3955);
    root.begin_compound("Metadata");
    root.int("RegionCount", 1);
    root.int("TotalBlocks", 1);
    root.int("TotalVolume", 8);
    root.begin_compound("EnclosingSize");
    root.int("x", 2);
    root.int("y", 2);
    root.int("z", 2);
    root.end_compound();
    root.end_compound(); // Metadata
    root.begin_compound("Regions");
    root.begin_compound("main");
    root.begin_compound("Position");
    root.int("x", 10);
    root.int("y", 20);
    root.int("z", 30);
    root.end_compound();
    // 三个轴都是负的：区域从 (10,20,30) 往负方向延伸 2 格。
    root.begin_compound("Size");
    root.int("x", -2);
    root.int("y", -2);
    root.int("z", -2);
    root.end_compound();
    root.begin_list("BlockStatePalette", 10, 2);
    root.string("Name", "minecraft:air");
    root.end_compound();
    root.string("Name", "minecraft:stone");
    root.end_compound();
    root.long_array("BlockStates", &longs);
    root.end_compound(); // main
    root.end_compound(); // Regions

    let schematic = parse(&root.gzip()).expect("parse");
    let region = &schematic.regions[0];
    assert_eq!(region.size, [2, 2, 2]);
    // min(10, 10 + (-1)) = 9，其余轴同理。
    assert_eq!(region.position, [9, 19, 29]);
}
