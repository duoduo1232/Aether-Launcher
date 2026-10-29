//! 从版本 jar 里提取方块资源，供 3D 预览做贴图渲染。
//!
//! # 为什么要读 jar
//!
//! 预览如果只给每个方块一个纯色，木板看不到纹理、半砖看不到斜面、玻璃看不到
//! 透明，整座建筑就糊成一片马赛克。真实的观感需要 Minecraft 自己的三样东西：
//!
//! - `assets/minecraft/blockstates/<方块>.json`：**方块状态 → 模型**的映射
//!   （朝向、半砖上下、连接形状……）；
//! - `assets/minecraft/models/block/<模型>.json`：模型的多边形盒子（`elements`），
//!   每个面带 UV 与贴图引用，可以带 `parent` 继承；
//! - `assets/minecraft/textures/block/<贴图>.png`：实际的像素。
//!
//! 这三样都在版本 jar 里（jar 就是 zip），所以不需要联网，也不需要玩家装模组。
//!
//! # 边界
//!
//! - jar 与其中的 JSON、PNG 都是**不可信输入**：条目数量、单个 JSON 大小、
//!   模型继承深度、PNG 尺寸都有上限，超限按「没有资源」处理而不是崩掉；
//! - 提取结果只留在应用数据目录，不改动玩家的游戏目录；
//! - 解析失败（老版本格式、模组方块）一律回退成「一个整方块 + 该方块的任一
//!   贴图」，保证预览永远画得出来。

use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

mod cache;
mod model;
mod pack;

pub use cache::{VersionJar, cache_key, list_version_jars, load_cached, store_cached};
pub use model::VariantBinding;
pub use pack::build_atlas;

/// 水面贴图的 id。含水方块要在自己的模型上叠一层这个贴图的水膜。
const WATER_STILL: &str = "minecraft:block/water_still";

/// 一个 jar 里允许读取的最大条目数，防止异常压缩包拖住解析。
const MAX_ENTRIES: usize = 40_000;

/// 单个 JSON（blockstate / model）的大小上限。
const MAX_JSON_BYTES: u64 = 4 * 1024 * 1024;

/// 单张贴图的大小上限。
const MAX_TEXTURE_BYTES: u64 = 4 * 1024 * 1024;

/// 模型 `parent` 链的最大深度。
const MAX_PARENT_DEPTH: usize = 16;

/// 一次提取允许处理的贴图数量。
const MAX_TEXTURES: usize = 4096;

#[derive(Debug)]
pub enum PackError {
    /// 打不开 jar。
    Io(std::io::Error),
    /// 不是合法的 zip，或缺少 `assets/minecraft`。
    NotAResourcePack,
    /// 条目过多。
    TooManyEntries,
}

impl std::fmt::Display for PackError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "{error}"),
            Self::NotAResourcePack => write!(formatter, "not a Minecraft version jar"),
            Self::TooManyEntries => write!(formatter, "the jar has too many entries"),
        }
    }
}

impl From<std::io::Error> for PackError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

/// 提取出来的原始资源，尚未烘焙成渲染用的模型。
#[derive(Debug, Default)]
pub struct RawPack {
    /// 方块 id（`minecraft:stone`）→ blockstate JSON 文本。
    pub blockstates: HashMap<String, String>,
    /// 模型 id（`minecraft:block/stairs`）→ model JSON 文本。
    pub models: HashMap<String, String>,
    /// 贴图 id（`minecraft:block/cherry_planks`）→ PNG 字节。
    pub textures: HashMap<String, Vec<u8>>,
}

/// 一份提取完成、可以直接给前端用的资源包。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetPack {
    /// 资源来自哪个版本 jar（只用于界面显示，不含绝对路径）。
    pub source_version: String,
    /// 方块 id → 该方块所有「状态 → 模型」的绑定。
    pub blocks: HashMap<String, Vec<VariantBinding>>,
    /// 贴图图集（PNG 字节，前端转成 data URL 或 Blob）。
    pub atlas_png: Vec<u8>,
    /// 图集里每一张贴图的 UV 区域，键是贴图 id。
    pub atlas: HashMap<String, AtlasSlot>,
    /// 图集边长（像素）。
    pub atlas_size: u32,
}

/// 图集里一个贴图的位置，坐标是 0..1 的 UV。
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AtlasSlot {
    pub u0: f32,
    pub v0: f32,
    pub u1: f32,
    pub v1: f32,
}

/// 从版本 jar 提取资源并烘焙成前端可用的资源包。
///
/// `jar` 必须是玩家自己 `.minecraft/versions/<版本>/<版本>.jar`，由后端在
/// 用户选择版本后解析得到；前端从不传路径。
pub fn build_asset_pack(jar: &Path, version_label: &str) -> Result<AssetPack, PackError> {
    let raw = read_jar(jar)?;
    if raw.blockstates.is_empty() || raw.models.is_empty() {
        return Err(PackError::NotAResourcePack);
    }

    let mut blocks = model::bake_all(&raw);

    // 图集只需要被模型引用到的贴图，外加一张水面贴图。
    //
    // 为什么要额外加水：含水方块（`waterlogged=true` 的楼梯、台阶、栅栏，
    // 以及海带、海草）在原版里除了自己的模型，还会叠一层流体。那层水用的
    // `water_still` 不一定被任何方块模型引用，不显式加进来就取不到 UV。
    let mut referenced = model::referenced_textures(&blocks);
    referenced.insert(WATER_STILL.to_owned());
    let built = build_atlas(&raw.textures, &referenced);

    // 按**贴图是否带透明像素**修正渲染层。
    //
    // 之前是按方块名列表猜（`glass`、`leaves`……），漏掉的方块（草、花、
    // 花瓣、模组方块）就被当成不透明，贴图里的透明背景直接画成黑色 ——
    // 画面上就是一片黑底卡片。贴图里有没有 alpha 是确定的事实，按它判断
    // 不需要维护任何名单。
    model::assign_layers_from_textures(&mut blocks, &built.transparent, &built.blended);

    // 带 `tintindex` 的面要乘对应颜色。树叶、草、水是三个不同的颜色，
    // 统一用草色会让樱花树叶变成灰绿。
    model::assign_tints(&mut blocks);
    // 烘焙阶段只知道贴图在图里的局部 UV；建好图集后补上格子偏移。
    model::apply_atlas_slots(&mut blocks, &built.slots);

    Ok(AssetPack {
        source_version: version_label.to_owned(),
        blocks,
        atlas_png: built.png,
        atlas: built.slots,
        atlas_size: built.side,
    })
}

/// 读取 jar 里的 `assets/minecraft/{blockstates,models,textures}`。
fn read_jar(jar: &Path) -> Result<RawPack, PackError> {
    let file = fs::File::open(jar)?;
    let mut archive = zip::ZipArchive::new(file).map_err(|_| PackError::NotAResourcePack)?;
    if archive.len() > MAX_ENTRIES {
        return Err(PackError::TooManyEntries);
    }

    let mut raw = RawPack::default();
    for index in 0..archive.len() {
        let Ok(mut entry) = archive.by_index(index) else {
            continue;
        };
        // zip 9 的 `name()` 返回 Result（文件名可能不是合法 UTF-8）。
        let Ok(entry_name) = entry.name() else {
            continue;
        };
        let Some(name) = entry_name
            .strip_prefix("assets/minecraft/")
            .map(str::to_owned)
        else {
            continue;
        };
        if entry.is_dir() {
            continue;
        }
        let size = entry.size();

        if let Some(rest) = name.strip_prefix("blockstates/") {
            if let Some(id) = rest.strip_suffix(".json")
                && id.len() <= 128
                && let Some(text) = read_json(&mut entry, size)
            {
                raw.blockstates.insert(format!("minecraft:{id}"), text);
            }
            continue;
        }
        if let Some(rest) = name.strip_prefix("models/") {
            if let Some(id) = rest.strip_suffix(".json")
                && id.len() <= 128
                && let Some(text) = read_json(&mut entry, size)
            {
                raw.models.insert(format!("minecraft:{id}"), text);
            }
            continue;
        }
        if let Some(rest) = name.strip_prefix("textures/")
            && let Some(id) = rest.strip_suffix(".png")
            && id.len() <= 128
            && raw.textures.len() < MAX_TEXTURES
            && size <= MAX_TEXTURE_BYTES
            && !id.contains("..")
        {
            let mut bytes = Vec::with_capacity(size as usize);
            if entry
                .take(MAX_TEXTURE_BYTES)
                .read_to_end(&mut bytes)
                .is_ok()
            {
                raw.textures.insert(format!("minecraft:{id}"), bytes);
            }
        }
    }

    Ok(raw)
}

/// 读一段不超过 [`MAX_JSON_BYTES`] 的文本。
fn read_json(entry: &mut impl Read, size: u64) -> Option<String> {
    if size > MAX_JSON_BYTES {
        return None;
    }
    let mut bytes = Vec::with_capacity(size as usize);
    entry.take(MAX_JSON_BYTES).read_to_end(&mut bytes).ok()?;
    String::from_utf8(bytes).ok()
}

/// 提取结果缓存的落地路径：`<root>/asset-packs/<版本 hash>/`。
#[must_use]
pub fn cache_dir(root: &Path, key: &str) -> PathBuf {
    root.join("asset-packs").join(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 用真实版本 jar 跑一遍提取：方块、模型、图集都要有内容。
    ///
    /// jar 路径来自环境变量，没有就跳过 —— CI 上没有游戏目录。
    #[test]
    fn extracts_a_real_version_jar() {
        let Some(jar) = std::env::var_os("AETHER_TEST_JAR") else {
            return;
        };
        let jar = std::path::PathBuf::from(jar);
        let pack = build_asset_pack(&jar, "test").expect("extract");

        assert!(!pack.blocks.is_empty(), "no blocks baked");
        assert!(!pack.atlas.is_empty(), "no textures in the atlas");
        assert!(pack.atlas_size >= 16);
        assert!(pack.atlas_png.starts_with(&[0x89, b'P', b'N', b'G']));

        // 一个几乎肯定存在的方块：石头。必须有几何且引用到贴图。
        let stone = pack.blocks.get("minecraft:stone").expect("stone baked");
        assert!(!stone.is_empty());
        assert!(!stone[0].model.quads.is_empty());
        assert!(!stone[0].model.texture_ids.is_empty());
    }

    /// 每个四边形的 UV 都必须落在它那张贴图的图集格子里。
    ///
    /// 这条盯的是「UV 只算到 0..1、没乘图集格子」的 bug —— 那时的表现是整个
    /// 画面采样到同一张乱码图。
    #[test]
    fn quads_sample_their_own_atlas_cell() {
        let Some(jar) = std::env::var_os("AETHER_TEST_JAR") else {
            return;
        };
        let pack = build_asset_pack(std::path::Path::new(&jar), "test").expect("extract");

        let mut checked = 0;
        for bindings in pack.blocks.values() {
            for binding in bindings {
                for quad in &binding.model.quads {
                    let Some((texture, _)) = &quad.uv else {
                        continue;
                    };
                    let Some(slot) = pack.atlas.get(texture) else {
                        continue;
                    };
                    for uv in &quad.uvs {
                        assert!(
                            uv[0] >= slot.u0 - 0.001
                                && uv[0] <= slot.u1 + 0.001
                                && uv[1] >= slot.v0 - 0.001
                                && uv[1] <= slot.v1 + 0.001,
                            "{texture}: uv {uv:?} outside slot {slot:?}"
                        );
                    }
                    checked += 1;
                }
            }
        }
        assert!(checked > 0, "no textured quads to check");
    }

    /// 图集格子不能越界，也不能空。
    #[test]
    fn atlas_slots_stay_inside_the_atlas() {
        let Some(jar) = std::env::var_os("AETHER_TEST_JAR") else {
            return;
        };
        let pack = build_asset_pack(std::path::Path::new(&jar), "test").expect("extract");
        for (id, slot) in &pack.atlas {
            assert!(slot.u0 >= 0.0 && slot.v0 >= 0.0, "{id} starts outside");
            assert!(slot.u1 <= 1.0 && slot.v1 <= 1.0, "{id} ends outside");
            assert!(slot.u0 < slot.u1 && slot.v0 < slot.v1, "{id} is empty");
        }
    }
    /// 每个四边形的 UV 必须落在它那张贴图的图集格子**内部**。
    ///
    /// 注意不能要求「必须落在格子的四个角」：模型的 `uv` 常常只取贴图的一小块
    /// （栅栏门用 `[0,7,2,9]` 这种局部矩形），四个角本来就落在格子中间。真正
    /// 要盯的是「有没有跑到别的格子去」。
    #[test]
    fn every_quad_uv_stays_inside_its_cell() {
        let Some(jar) = std::env::var_os("AETHER_TEST_JAR") else {
            return;
        };
        let pack = build_asset_pack(std::path::Path::new(&jar), "test").expect("extract");
        let mut checked = 0;
        for (block, bindings) in &pack.blocks {
            for binding in bindings {
                for quad in &binding.model.quads {
                    let Some((texture, _)) = &quad.uv else {
                        continue;
                    };
                    let Some(slot) = pack.atlas.get(texture) else {
                        continue;
                    };
                    // 留一个像素的余量：最近邻采样在边界上会轻微溢出。
                    let margin = 1.0 / 512.0;
                    for uv in &quad.uvs {
                        assert!(
                            uv[0] >= slot.u0 - margin
                                && uv[0] <= slot.u1 + margin
                                && uv[1] >= slot.v0 - margin
                                && uv[1] <= slot.v1 + margin,
                            "{block}: uv {uv:?} escaped cell {slot:?}"
                        );
                    }
                    checked += 1;
                }
            }
        }
        assert!(checked > 0, "no quads to check");
    }

    /// 每个面的顶点从外侧看必须是逆时针，否则会被当背面剔除或贴图镜像。
    #[test]
    fn quad_winding_faces_outward() {
        let Some(jar) = std::env::var_os("AETHER_TEST_JAR") else {
            return;
        };
        let pack = build_asset_pack(std::path::Path::new(&jar), "test").expect("extract");

        let mut checked = 0;
        for (block, bindings) in &pack.blocks {
            for binding in bindings {
                for quad in &binding.model.quads {
                    let [p0, p1, p2, _] = quad.positions;
                    let edge1 = [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]];
                    let edge2 = [p2[0] - p0[0], p2[1] - p0[1], p2[2] - p0[2]];
                    let cross = [
                        edge1[1] * edge2[2] - edge1[2] * edge2[1],
                        edge1[2] * edge2[0] - edge1[0] * edge2[2],
                        edge1[0] * edge2[1] - edge1[1] * edge2[0],
                    ];
                    let dot = cross[0] * quad.normal[0]
                        + cross[1] * quad.normal[1]
                        + cross[2] * quad.normal[2];
                    // 零面积的四边形（模型里的退化面）没有朝向可言，跳过。
                    let area =
                        (cross[0] * cross[0] + cross[1] * cross[1] + cross[2] * cross[2]).sqrt();
                    if area < 1e-9 {
                        continue;
                    }
                    assert!(
                        dot > 0.0,
                        "winding points away from the normal: block {block} p0 {p0:?} p1 {p1:?} p2 {p2:?} cross {cross:?} normal {:?}",
                        quad.normal
                    );
                    checked += 1;
                }
            }
        }
        assert!(checked > 0, "no quads to check");
    }
}

#[cfg(test)]
mod probe {
    /// 打印真实 jar 的提取规模，人工核对一次。
    #[test]
    fn report_real_jar_scale() {
        let Some(jar) = std::env::var_os("AETHER_TEST_JAR") else {
            return;
        };
        let pack = super::build_asset_pack(std::path::Path::new(&jar), "probe").expect("extract");
        let quads: usize = pack
            .blocks
            .values()
            .flat_map(|bindings| bindings.iter())
            .map(|binding| binding.model.quads.len())
            .sum();
        println!(
            "blocks={} atlas_entries={} atlas_side={} quads={} atlas_png_bytes={}",
            pack.blocks.len(),
            pack.atlas.len(),
            pack.atlas_size,
            quads,
            pack.atlas_png.len()
        );

        // 索引色贴图必须真的被解码。如果解码器吐洋红占位，图集里会有大量
        // 纯洋红像素 —— 那正是「一片乱码」的画面来源。
        if let Some(rgba) = super::pack::decode_for_test(&pack.atlas_png) {
            let total = rgba.len() / 4;
            let magenta = rgba
                .as_chunks::<4>()
                .0
                .iter()
                .filter(|pixel| pixel[0] > 240 && pixel[1] < 20 && pixel[2] > 240)
                .count();
            println!("atlas magenta pixels: {magenta} / {total}");
            assert!(
                magenta * 100 < total,
                "atlas is mostly magenta ({magenta}/{total}): indexed textures are not decoded"
            );
        }
        for name in [
            "minecraft:stone",
            "minecraft:cherry_planks",
            "minecraft:oak_stairs",
        ] {
            if let Some(bindings) = pack.blocks.get(name) {
                let quad = &bindings[0].model.quads[0];
                println!(
                    "{name}: quads={} p0={:?} uv0={:?} tex={:?}",
                    bindings[0].model.quads.len(),
                    quad.positions[0],
                    quad.uvs[0],
                    bindings[0].model.texture_ids
                );
            }
        }
    }

    /// 诊断用：打印方向明确的方块（工作台）六个面的顶点与 UV。
    ///
    /// 工作台北面是 front、东面是 side、上面是 top，贴图各不相同。打出顶点与
    /// UV 就能人工核对「左上角配的是 (u0,v0) 还是 (u0,v1)」。
    #[test]
    fn report_a_directional_block() {
        let Some(jar) = std::env::var_os("AETHER_TEST_JAR") else {
            return;
        };
        let pack = super::build_asset_pack(std::path::Path::new(&jar), "probe").expect("extract");
        let Some(bindings) = pack.blocks.get("minecraft:crafting_table") else {
            println!("crafting_table not baked");
            return;
        };
        for quad in &bindings[0].model.quads {
            let [p0, p1, p2, p3] = quad.positions;
            let [u0, u1, u2, u3] = quad.uvs;
            println!(
                "\ntex={:?} n={:?}\n  p0={p0:?} uv={u0:?}\n  p1={p1:?} uv={u1:?}\n  p2={p2:?} uv={u2:?}\n  p3={p3:?} uv={u3:?}",
                quad.uv, quad.normal
            );
        }
    }

    /// 花草的十字模型必须真的转出斜面。
    /// `block/cross` 的两个 element 都带 `rotation: {axis: y, angle: 45}`。
    /// 漏掉这个旋转，十字就退化成一张贴在方块中间的薄片，画面上是一堆
    /// 黑底图标。
    #[test]
    fn a_cross_model_is_rotated_into_an_x() {
        let Some(jar) = std::env::var_os("AETHER_TEST_JAR") else {
            return;
        };
        let pack = super::build_asset_pack(std::path::Path::new(&jar), "probe").expect("extract");
        let Some(bindings) = pack
            .blocks
            .get("minecraft:dandelion")
            .or_else(|| pack.blocks.get("minecraft:poppy"))
            .or_else(|| pack.blocks.get("minecraft:short_grass"))
        else {
            println!("no flower baked");
            return;
        };

        // 旋转 45° 之后，面的法线不再是轴对齐的。
        let mut tilted = 0;
        for quad in &bindings[0].model.quads {
            let [nx, ny, nz] = quad.normal;
            if nx.abs() > 0.1 && nz.abs() > 0.1 && ny.abs() < 0.1 {
                tilted += 1;
            }
        }
        println!("tilted quads: {tilted}/{}", bindings[0].model.quads.len());
        assert!(tilted > 0, "the cross model was not rotated");
    }

    /// 楼梯的 multipart 条件必须带出来，否则每个变体都会被画一遍。
    ///
    /// `oak_stairs` 的 blockstate 是 multipart：`facing`/`half`/`shape` 组合出
    /// 直梯、内转角、外转角等十几个部件。条件丢掉时画面就是「所有楼梯都是
    /// 转角楼梯」。
    #[test]
    fn stairs_keep_their_multipart_conditions() {
        let Some(jar) = std::env::var_os("AETHER_TEST_JAR") else {
            return;
        };
        let pack = super::build_asset_pack(std::path::Path::new(&jar), "probe").expect("extract");
        let Some(bindings) = pack.blocks.get("minecraft:oak_stairs") else {
            println!("oak_stairs not baked");
            return;
        };

        let conditional = bindings.iter().filter(|b| !b.when.is_empty()).count();
        let straight = bindings
            .iter()
            .filter(|b| {
                b.when
                    .iter()
                    .any(|(key, value)| key == "shape" && value == "straight")
            })
            .count();
        println!(
            "oak_stairs bindings={} conditional={conditional} straight={straight}",
            bindings.len()
        );

        assert!(
            conditional > 0,
            "multipart 条件被丢掉了，楼梯会画出所有变体"
        );
        assert!(straight > 0, "没有直梯部件，只剩转角");
    }

    /// 对比一个真实投影用到的方块和材质包能提供的方块，打印对不上的那些。
    ///
    /// 用 `AETHER_TEST_SCHEMATIC` 指定投影、`AETHER_TEST_JAR` 指定材质 jar。
    #[test]
    fn report_blocks_the_pack_cannot_texture() {
        let (Some(jar), Some(schematic)) = (
            std::env::var_os("AETHER_TEST_JAR"),
            std::env::var_os("AETHER_TEST_SCHEMATIC"),
        ) else {
            return;
        };
        let pack = super::build_asset_pack(std::path::Path::new(&jar), "probe").expect("extract");
        let bytes = std::fs::read(&schematic).expect("read schematic");
        let parsed = aether_schematic::parse(&bytes).expect("parse schematic");

        let mut used: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for region in &parsed.regions {
            for entry in &region.palette {
                let name = entry.split('[').next().unwrap_or(entry).trim();
                if name.is_empty() || name.ends_with(":air") || name == "minecraft:air" {
                    continue;
                }
                used.insert(name.to_owned());
            }
        }

        let mut missing: Vec<&String> = used
            .iter()
            .filter(|name| !pack.blocks.contains_key(*name))
            .collect();
        missing.sort();
        println!(
            "project_blocks={} pack_blocks={} missing={}",
            used.len(),
            pack.blocks.len(),
            missing.len()
        );
        for name in missing.iter().take(200) {
            println!("  missing: {name}");
        }
    }

    /// 直接看一眼某个方块有没有被烘焙出来，以及它的面去了哪一层。
    #[test]
    fn report_one_block() {
        let (Some(jar), name) = (
            std::env::var_os("AETHER_TEST_JAR"),
            std::env::var("AETHER_TEST_BLOCK").unwrap_or_else(|_| "minecraft:glass".to_owned()),
        ) else {
            return;
        };
        let pack = super::build_asset_pack(std::path::Path::new(&jar), "probe").expect("extract");
        match pack.blocks.get(&name) {
            Some(bindings) => {
                println!("{name}: bindings={}", bindings.len());
                for binding in bindings.iter().take(3) {
                    println!(
                        "  when={:?} rot=({},{}) quads={}",
                        binding.when,
                        binding.rotation_x,
                        binding.rotation_y,
                        binding.model.quads.len()
                    );
                    for quad in binding.model.quads.iter().take(2) {
                        println!("    layer={:?} tex={:?}", quad.layer, quad.uv);
                    }
                }
            }
            None => println!("{name}: NOT BAKED"),
        }
    }
}
