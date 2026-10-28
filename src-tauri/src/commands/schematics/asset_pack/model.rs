//! 把 blockstate + model 烘焙成「方块 → 若干带 UV 的四边形」。
//!
//! Minecraft 的模型系统有两层：
//!
//! 1. **blockstate**：`variants` 把一组属性（`facing=north,half=top`）映射到
//!    一个模型，并带 `x`/`y` 旋转；`multipart` 则按条件叠加多个部件；
//! 2. **model**：`elements` 是一堆立方体盒子（`from`/`to`，单位 1/16 方块），
//!    每个盒子六个面各自带 `uv`、贴图引用和 `cullface`；`parent` 用于继承，
//!    子模型覆盖父模型的贴图与元素。
//!
//! 这里只保留预览需要的信息：每个面落到图集里哪一块 UV、法线方向、是否
//! `cullface`（被邻居挡住就能整面跳过），以及所属渲染层。

use std::collections::{HashMap, HashSet};

use super::{MAX_PARENT_DEPTH, RawPack};
use serde::{Deserialize, Serialize};

/// 立方体的六个面方向。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Face {
    Down,
    Up,
    North,
    South,
    West,
    East,
}

const ALL_FACES: [Face; 6] = [
    Face::Down,
    Face::Up,
    Face::North,
    Face::South,
    Face::West,
    Face::East,
];

/// 渲染层：决定是否需要透明混合。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RenderLayer {
    Opaque,
    /// 二值透明（树叶、铁栏杆）：alpha < 0.5 直接丢弃。
    Cutout,
    /// 半透明（染色玻璃、冰）。
    Translucent,
}

/// 一个面。坐标已归一化到 0..1 的方块空间。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Quad {
    /// 四个顶点，逆时针（从外面看）。
    pub positions: [[f32; 3]; 4],
    /// 四个顶点在这张贴图**自身**里的 UV（0..1）。图集偏移由
    /// [`apply_atlas_slots`] 补上。
    pub uvs: [[f32; 2]; 4],
    /// 法线，用于面剔除与光照。
    pub normal: [f32; 3],
    /// 该面声明的 `cullface`：邻居占住这个方向时可以整面跳过。
    pub cullface: Option<Face>,
    pub layer: RenderLayer,
    /// 这个面用哪张贴图（`minecraft:block/oak_planks`），同时也是图集的键。
    pub uv: Option<(String, String)>,
}

/// 一个模型烘焙后的几何。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BakedModel {
    pub quads: Vec<Quad>,
    /// 这个模型用到的贴图 id（图集只需要这些）。
    pub texture_ids: Vec<String>,
    /// 整个模型是否填满 1×1×1（决定邻居能不能挡住它）。
    pub full_cube: bool,
}

/// 一个方块状态绑定。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VariantBinding {
    /// 属性组合，如 `[["facing", "north"]]`；空表示「总是适用」。
    pub when: Vec<(String, String)>,
    pub rotation_x: i32,
    pub rotation_y: i32,
    pub model: BakedModel,
}

/// 解析后的一个面（尚未落到图集）。
#[derive(Debug, Clone)]
struct RawFace {
    uv: [f32; 4],
    texture: String,
    cullface: Option<Face>,
}

/// 解析后的一个盒子。
#[derive(Debug, Clone)]
struct RawElement {
    from: [f32; 3],
    to: [f32; 3],
    faces: Vec<(Face, RawFace)>,
}

impl RawElement {
    /// 这个盒子是不是正好填满整个方块（`from = 0,0,0` 且 `to = 16,16,16`）。
    fn is_full_cube(&self) -> bool {
        self.from == [0.0; 3] && self.to == [16.0; 3]
    }
}

/// 一个模型解析完（已合并父链）的样子。
#[derive(Debug, Clone, Default)]
struct ResolvedModel {
    elements: Vec<RawElement>,
    textures: HashMap<String, String>,
}

/// 把所有 blockstate 烘焙成前端可用的形式。
pub fn bake_all(raw: &RawPack) -> HashMap<String, Vec<VariantBinding>> {
    let mut cache: HashMap<String, ResolvedModel> = HashMap::new();
    let mut blocks = HashMap::new();

    for (block_id, json) in &raw.blockstates {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
            continue;
        };
        let layer = layer_for(block_id);
        let bindings = bake_block(&value, raw, &mut cache, layer);
        if !bindings.is_empty() {
            blocks.insert(block_id.clone(), bindings);
        }
    }

    blocks
}

/// 收集所有被模型引用到的贴图 id，图集只需要这些。
pub fn referenced_textures(blocks: &HashMap<String, Vec<VariantBinding>>) -> HashSet<String> {
    let mut used = HashSet::new();
    for bindings in blocks.values() {
        for binding in bindings {
            used.extend(binding.model.texture_ids.iter().cloned());
        }
    }
    used
}

/// 一个方块应该用哪个渲染层。
///
/// jar 的模型 JSON 里没有可靠的渲染层信息（原版由硬编码的 block 类决定），
/// 所以按方块名判断：玻璃、冰、粘液块这类要混合；树叶、花草、铁栏杆这类只
/// 需要二值透明。认不出来的按不透明处理，代价是玻璃背后会挡一层。
fn layer_for(block_id: &str) -> RenderLayer {
    let name = block_id.strip_prefix("minecraft:").unwrap_or(block_id);

    // 半透明：真的要看穿过去。
    const TRANSLUCENT: [&str; 8] = [
        "glass",
        "glass_pane",
        "tinted_glass",
        "ice",
        "frosted_ice",
        "slime_block",
        "honey_block",
        "water",
    ];
    if TRANSLUCENT.contains(&name) || name.starts_with("stained_glass") {
        return RenderLayer::Translucent;
    }

    // 二值透明：alpha 低于阈值直接丢弃。
    const CUTOUT: [&str; 12] = [
        "leaves",
        "vine",
        "ladder",
        "torch",
        "wall_torch",
        "soul_torch",
        "soul_wall_torch",
        "iron_bars",
        "chain",
        "cobweb",
        "scaffolding",
        "flower_pot",
    ];
    if CUTOUT.contains(&name)
        || name.ends_with("_leaves")
        || name.ends_with("_door")
        || name.ends_with("_trapdoor")
        || name.ends_with("_sapling")
        || name.ends_with("_pane")
        || name.ends_with("_carpet")
    {
        return RenderLayer::Cutout;
    }

    RenderLayer::Opaque
}

/// 把一个 blockstate JSON 烘焙成若干绑定。
fn bake_block(
    value: &serde_json::Value,
    raw: &RawPack,
    cache: &mut HashMap<String, ResolvedModel>,
    layer: RenderLayer,
) -> Vec<VariantBinding> {
    let mut out = Vec::new();

    if let Some(variants) = value.get("variants").and_then(|v| v.as_object()) {
        for (key, entry) in variants {
            let when = parse_when(key);
            if let Some(binding) = bake_entry(first_of(entry), when, raw, cache, layer) {
                out.push(binding);
            }
        }
        return out;
    }

    if let Some(parts) = value.get("multipart").and_then(|v| v.as_array()) {
        // 预览不知道方块实际属性，把每个部件都当成「总是适用」的绑定：
        // 会多画几个部件，但不至于整个方块消失。
        for part in parts {
            let entry = part.get("apply").unwrap_or(part);
            if let Some(binding) = bake_entry(first_of(entry), Vec::new(), raw, cache, layer) {
                out.push(binding);
            }
        }
    }

    out
}

/// `{"model": "...", "x": 90}` 这种条目 → 绑定。
fn bake_entry(
    entry: &serde_json::Value,
    when: Vec<(String, String)>,
    raw: &RawPack,
    cache: &mut HashMap<String, ResolvedModel>,
    layer: RenderLayer,
) -> Option<VariantBinding> {
    let model_id = qualify(entry.get("model")?.as_str()?);
    let resolved = resolve_model(&model_id, raw, cache, 0)?;

    let rotation_x = entry
        .get("x")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(0) as i32;
    let rotation_y = entry
        .get("y")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(0) as i32;

    let mut quads = Vec::new();
    let mut texture_ids = HashSet::new();
    for element in &resolved.elements {
        for (face, raw_face) in &element.faces {
            // `#side` 这种引用要顺着模型的 textures 表解成真实贴图 id。
            let Some(texture) = resolve_texture(&raw_face.texture, &resolved.textures) else {
                continue;
            };
            texture_ids.insert(texture.clone());
            let mut quad = build_quad(element, *face, raw_face, layer);
            quad.uv = Some((texture.clone(), texture));
            quads.push(quad);
        }
    }

    // 容错：没有可用元素时回退成整方块，用模型里的任意一张贴图。
    if quads.is_empty() {
        let texture = resolved
            .textures
            .values()
            .filter_map(|reference| resolve_texture(reference, &resolved.textures))
            .next()?;
        let fallback_face = RawFace {
            uv: [0.0, 0.0, 16.0, 16.0],
            texture: "#all".to_owned(),
            cullface: None,
        };
        let cube = RawElement {
            from: [0.0; 3],
            to: [16.0; 3],
            faces: ALL_FACES
                .iter()
                .map(|face| (*face, fallback_face.clone()))
                .collect(),
        };
        let mut textures = resolved.textures.clone();
        textures.insert("all".to_owned(), texture.clone());
        for face in ALL_FACES {
            let mut quad = build_quad(&cube, face, &fallback_face, layer);
            quad.uv = Some((texture.clone(), texture.clone()));
            quads.push(quad);
        }
        texture_ids.insert(texture);
    }

    Some(VariantBinding {
        when,
        rotation_x,
        rotation_y,
        model: BakedModel {
            quads,
            texture_ids: texture_ids.into_iter().collect(),
            full_cube: resolved.elements.len() == 1 && resolved.elements[0].is_full_cube(),
        },
    })
}

/// 解析模型及其父链，子覆盖父。
fn resolve_model(
    id: &str,
    raw: &RawPack,
    cache: &mut HashMap<String, ResolvedModel>,
    depth: usize,
) -> Option<ResolvedModel> {
    if let Some(cached) = cache.get(id) {
        return Some(cached.clone());
    }
    if depth > MAX_PARENT_DEPTH {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(raw.models.get(id)?).ok()?;

    let mut resolved = match value.get("parent").and_then(|p| p.as_str()) {
        Some(parent) => resolve_model(&qualify(parent), raw, cache, depth + 1).unwrap_or_default(),
        None => ResolvedModel::default(),
    };

    if let Some(textures) = value.get("textures").and_then(|t| t.as_object()) {
        for (key, entry) in textures {
            if let Some(reference) = entry.as_str() {
                resolved.textures.insert(key.clone(), reference.to_owned());
            }
        }
    }

    // 出现 `elements` 就是整体替换，和 Minecraft 的语义一致。
    if let Some(elements) = value.get("elements").and_then(|e| e.as_array()) {
        resolved.elements = elements.iter().filter_map(parse_element).collect();
    }

    cache.insert(id.to_owned(), resolved.clone());
    Some(resolved)
}

/// 一个 `elements` 条目。
fn parse_element(value: &serde_json::Value) -> Option<RawElement> {
    let from = parse_vec3(value.get("from")?)?;
    let to = parse_vec3(value.get("to")?)?;
    let faces_object = value.get("faces")?.as_object()?;

    let mut faces = Vec::new();
    for (name, face_value) in faces_object {
        let Some(face) = parse_face_name(name) else {
            continue;
        };
        let Some(texture) = face_value.get("texture").and_then(|t| t.as_str()) else {
            continue;
        };
        let uv = face_value
            .get("uv")
            .and_then(|uv| uv.as_array())
            .and_then(|uv| {
                if uv.len() != 4 {
                    return None;
                }
                Some([
                    uv[0].as_f64()? as f32,
                    uv[1].as_f64()? as f32,
                    uv[2].as_f64()? as f32,
                    uv[3].as_f64()? as f32,
                ])
            })
            .unwrap_or([0.0, 0.0, 16.0, 16.0]);
        let cullface = face_value
            .get("cullface")
            .and_then(|c| c.as_str())
            .and_then(parse_face_name);
        faces.push((
            face,
            RawFace {
                uv,
                texture: texture.to_owned(),
                cullface,
            },
        ));
    }

    if faces.is_empty() {
        return None;
    }
    Some(RawElement { from, to, faces })
}

/// 把一个面变成四边形。
///
/// UV 先按「贴图自身」归一化（`uv` 里是 0..16 的像素坐标），图集偏移之后由
/// [`apply_atlas_slots`] 统一补上。这样同一个模型换个图集位置不用重新烘焙。
fn build_quad(element: &RawElement, face: Face, raw_face: &RawFace, layer: RenderLayer) -> Quad {
    let from = element.from.map(|value| value / 16.0);
    let to = element.to.map(|value| value / 16.0);

    let [u0, v0, u1, v1] = raw_face.uv;
    let uvs = [
        [u0 / 16.0, v1 / 16.0],
        [u1 / 16.0, v1 / 16.0],
        [u1 / 16.0, v0 / 16.0],
        [u0 / 16.0, v0 / 16.0],
    ];

    Quad {
        positions: face_corners(face, from, to),
        uvs,
        normal: face_normal(face),
        cullface: raw_face.cullface,
        layer,
        uv: None,
    }
}

/// 把烘焙阶段的 0..1 局部 UV 映射到图集里的实际格子。
///
/// 烘焙时只知道「这张贴图在这个面里占 0..1 的哪一段」，不知道它在图集第几格；
/// 图集建好后由这里补上偏移与缩放。**不做这一步，每个面都会采样整张图集**，
/// 画面上就是一片乱码。
pub(super) fn apply_atlas_slots(
    blocks: &mut HashMap<String, Vec<VariantBinding>>,
    atlas: &HashMap<String, super::AtlasSlot>,
) {
    for bindings in blocks.values_mut() {
        for binding in bindings.iter_mut() {
            for quad in &mut binding.model.quads {
                let Some((texture, _)) = &quad.uv else {
                    continue;
                };
                let Some(slot) = atlas.get(texture) else {
                    continue;
                };
                let (u0, v0) = (slot.u0, slot.v0);
                let (du, dv) = (slot.u1 - slot.u0, slot.v1 - slot.v0);
                for uv in &mut quad.uvs {
                    uv[0] = u0 + uv[0] * du;
                    uv[1] = v0 + uv[1] * dv;
                }
            }
        }
    }
}

/// 六个面的四个角，逆时针（从外面看）。
fn face_corners(face: Face, from: [f32; 3], to: [f32; 3]) -> [[f32; 3]; 4] {
    let [x0, y0, z0] = from;
    let [x1, y1, z1] = to;
    match face {
        Face::Down => [[x0, y0, z0], [x1, y0, z0], [x1, y0, z1], [x0, y0, z1]],
        Face::Up => [[x0, y1, z1], [x1, y1, z1], [x1, y1, z0], [x0, y1, z0]],
        Face::North => [[x1, y0, z0], [x0, y0, z0], [x0, y1, z0], [x1, y1, z0]],
        Face::South => [[x0, y0, z1], [x1, y0, z1], [x1, y1, z1], [x0, y1, z1]],
        Face::West => [[x0, y0, z0], [x0, y0, z1], [x0, y1, z1], [x0, y1, z0]],
        Face::East => [[x1, y0, z1], [x1, y0, z0], [x1, y1, z0], [x1, y1, z1]],
    }
}

fn face_normal(face: Face) -> [f32; 3] {
    match face {
        Face::Down => [0.0, -1.0, 0.0],
        Face::Up => [0.0, 1.0, 0.0],
        Face::North => [0.0, 0.0, -1.0],
        Face::South => [0.0, 0.0, 1.0],
        Face::West => [-1.0, 0.0, 0.0],
        Face::East => [1.0, 0.0, 0.0],
    }
}

fn parse_face_name(name: &str) -> Option<Face> {
    match name {
        "down" => Some(Face::Down),
        "up" => Some(Face::Up),
        "north" => Some(Face::North),
        "south" => Some(Face::South),
        "west" => Some(Face::West),
        "east" => Some(Face::East),
        _ => None,
    }
}

fn parse_vec3(value: &serde_json::Value) -> Option<[f32; 3]> {
    let array = value.as_array()?;
    if array.len() != 3 {
        return None;
    }
    Some([
        array[0].as_f64()? as f32,
        array[1].as_f64()? as f32,
        array[2].as_f64()? as f32,
    ])
}

/// `facing=north,half=top` → `[("facing","north"), ("half","top")]`。
fn parse_when(key: &str) -> Vec<(String, String)> {
    if key.is_empty() {
        return Vec::new();
    }
    key.split(',')
        .filter_map(|pair| pair.split_once('='))
        .map(|(name, value)| (name.trim().to_owned(), value.trim().to_owned()))
        .collect()
}

/// variant 值可能是数组（随机模型），取第一个。
fn first_of(value: &serde_json::Value) -> &serde_json::Value {
    match value.as_array() {
        Some(array) => array.first().unwrap_or(value),
        None => value,
    }
}

/// `block/stairs` → `minecraft:block/stairs`。
fn qualify(id: &str) -> String {
    if id.contains(':') {
        id.to_owned()
    } else {
        format!("minecraft:{id}")
    }
}

/// 把 `#side` 顺着 textures 表解成真实贴图 id。
///
/// 返回值**带命名空间**（`minecraft:block/stone`），因为 jar 里读出来的贴图
/// 就按这个键存放 —— 剥掉前缀会让图集一张都找不到。
fn resolve_texture(reference: &str, textures: &HashMap<String, String>) -> Option<String> {
    let mut current = reference;
    for _ in 0..8 {
        if let Some(key) = current.strip_prefix('#') {
            current = textures.get(key)?;
            continue;
        }
        return Some(qualify(current));
    }
    None
}
