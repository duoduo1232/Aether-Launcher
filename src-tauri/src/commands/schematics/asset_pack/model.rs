//! 把 blockstate + model 烘焙成「方块 → 若干带 UV 的四边形」。
//!
//! Minecraft 的模型系统有两层：
//!
//! 1. **blockstate**：`variants` 把一组属性（`facing=north,half=top`）映射到
//!    一个模型，并带 `x`/`y` 旋转与 `uvlock`；`multipart` 则按条件叠加多个部件；
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
    /// 顶点色。`[1,1,1]` 表示不染色；草和树叶按群系色相乘。
    pub tint: [f32; 3],
    /// 是否按朝向变暗（原版 `shade`，element 级，缺省 `true`）。
    ///
    /// 十字花草、灯笼提手写的是 `"shade": false`：它们斜着放，按朝向打暗只会
    /// 让一株草两面明暗不一。
    #[serde(default = "default_true")]
    pub shade: bool,
}

/// 一个模型烘焙后的几何。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BakedModel {
    pub quads: Vec<Quad>,
    /// 这个模型用到的贴图 id（图集只需要这些）。
    pub texture_ids: Vec<String>,
    /// 整个模型是否填满 1×1×1（决定邻居能不能挡住它）。
    pub full_cube: bool,
    /// 是否做环境光遮蔽（原版 `ambientocclusion`，沿父链继承，缺省 `true`）。
    ///
    /// `block/cross` 等植物模型是 `false`。不看这个标志的话，草丛底部会被
    /// 下面的草方块压暗、顶部却是亮的，一片草地就是脏兮兮的渐变。
    #[serde(default = "default_true")]
    pub ambient_occlusion: bool,
}

fn default_true() -> bool {
    true
}

/// 一个方块状态绑定。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VariantBinding {
    /// 属性组合，如 `[["facing", "north"]]`；空表示「总是适用」。
    pub when: Vec<(String, String)>,
    /**
     * 这条绑定是否「独占」。
     *
     * blockstate 有两种写法，语义不同：
     *
     * - `variants`：每个属性组合对应**一个**模型，多个都能匹配时只取最精确的
     *   那一个。标 `true`。
     * - `multipart`：多个部件**可以叠加**（楼梯 = 底座 + 踏板）。标 `false`。
     *
     * 之前不区分，一律「全部命中」。`cherry_shelf` 那种「无条件 facing 变体 +
     * 带 powered 的变体」的双层写法，就会把两套模型一起画出来。
     */
    pub exclusive: bool,
    pub rotation_x: i32,
    pub rotation_y: i32,
    pub model: BakedModel,
}

/// 平原群系的草色，`#91BD59`。带 `tintindex` 的面乘上它。
///
/// 原版按生物群系取色，投影里没有群系信息，固定成平原色就够了。
const GRASS_TINT: [f32; 3] = [0.569, 0.741, 0.349];

/// 平原群系的树叶色，`#48B518`。
///
/// 和草色是**两个**值，树叶明显更绿。统一用草色会让樱花树叶变成一坨灰绿。
const FOLIAGE_TINT: [f32; 3] = [0.282, 0.71, 0.094];

/// 水面色，`#3F76E4`。
const WATER_TINT: [f32; 3] = [0.247, 0.463, 0.894];

/// 某个方块带 `tintindex` 的面应该乘什么颜色。
///
/// 三类方块的着色各不相同：树叶用树叶色，水用水色，其余（草方块、高草、
/// 甘蔗）用草色。混用会让樱花树叶变成灰绿。
fn tint_for_block(block_id: &str) -> [f32; 3] {
    let name = block_id.strip_prefix("minecraft:").unwrap_or(block_id);
    // 樱花、杜鹃这类树叶的贴图本身就是彩色（粉色、带花），再乘一层树叶色
    // 会变成浑浊的绿。原版对它们不染色，这里按名字放行。
    if name.contains("cherry_leaves") || name.contains("azalea_leaves") {
        return [1.0, 1.0, 1.0];
    }
    if name.contains("leaves") || name.contains("vine") {
        return FOLIAGE_TINT;
    }
    if name.contains("water") {
        return WATER_TINT;
    }
    GRASS_TINT
}

/// 这些方块**无论**模型里有没有 `tintindex` 都要染色。
///
/// 照 shulkr 的 `Xt()`：`e === 'water' || t.waterlogged === 'true'` 时直接用
/// 水色。原版的水不是按模型画的（`block/water.json` 里连 `elements` 都没有），
/// 靠渲染器的流体几何上色，所以它的 `tintindex` 这条路根本不存在 —— 只能
/// 按方块名认。`waterlogged` 的方块（含水楼梯、含水台阶）同理。
fn is_always_tinted(block_id: &str) -> bool {
    let name = block_id.strip_prefix("minecraft:").unwrap_or(block_id);
    name == "water" || name == "flowing_water" || name == "bubble_column"
}

/// 解析后的一个面（尚未落到图集）。
#[derive(Debug, Clone)]
struct RawFace {
    /// 面级 UV（0..16 像素）。`None` 表示模型没写，按面朝向从 `from`/`to` 推导。
    uv: Option<[f32; 4]>,
    texture: String,
    cullface: Option<Face>,
    /// 模型里声明了 `tintindex` 的面要按生物群系染色（草的顶面、树叶）。
    tinted: bool,
    /// 面级 UV 旋转（0/90/180/270）。楼梯的侧面、门的贴图靠它摆正，
    /// 漏掉就让贴图整体转了 90°，看起来"贴图错了"。
    rotation: i32,
}

/// 解析后的一个盒子。
#[derive(Debug, Clone)]
struct RawElement {
    from: [f32; 3],
    to: [f32; 3],
    faces: Vec<(Face, RawFace)>,
    /// 元素自身的旋转（`rotation` 字段）。花草的十字模型就是靠它绕 Y 转 45°。
    rotation: Option<ElementRotation>,
    /// `elements[].shade`，缺省 `true`。
    shade: bool,
}

/// `elements[].rotation`：绕某个轴、以 `origin` 为心旋转 `angle` 度。
#[derive(Debug, Clone, Copy)]
struct ElementRotation {
    origin: [f32; 3],
    axis: RotationAxis,
    angle: f32,
    /// `rescale: true` 时把盒子拉伸回原来的外接尺寸（原版用于让斜置的
    /// 十字模型仍然填满方块）。
    rescale: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RotationAxis {
    X,
    Y,
    Z,
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
    /// `ambientocclusion`。`None` 表示整条父链都没写，按原版缺省 `true`。
    ambient_occlusion: Option<bool>,
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
        let bindings = bake_block(block_id, &value, raw, &mut cache, layer);
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
    block_id: &str,
    value: &serde_json::Value,
    raw: &RawPack,
    cache: &mut HashMap<String, ResolvedModel>,
    layer: RenderLayer,
) -> Vec<VariantBinding> {
    let mut out = Vec::new();

    if let Some(variants) = value.get("variants").and_then(|v| v.as_object()) {
        for (key, entry) in variants {
            let when = parse_when(key);
            // `variants` 语义：每个组合一个模型，命中多个时只取最精确的一个。
            if let Some(binding) =
                bake_entry(block_id, first_of(entry), when, raw, cache, layer, true)
            {
                out.push(binding);
            }
        }
        return out;
    }

    if let Some(parts) = value.get("multipart").and_then(|v| v.as_array()) {
        // 每个部件自带 `when` 条件（楼梯的 `facing`/`half`/`shape`、栅栏的
        // `north`/`south`…）。条件必须带出去：带到前端后按方块的真实属性筛选，
        // 只画命中的那几个部件。不带条件会把十几个变体全叠在一起，看起来就是
        // 「所有楼梯都成了转角楼梯」。
        for part in parts {
            let entry = part.get("apply").unwrap_or(part);
            let when = part
                .get("when")
                .map(parse_when_condition)
                .unwrap_or_default();
            // `multipart` 语义：多个部件可以叠加。
            if let Some(binding) =
                bake_entry(block_id, first_of(entry), when, raw, cache, layer, false)
            {
                out.push(binding);
            }
        }
    }

    out
}

/// `{"model": "...", "x": 90}` 这种条目 → 绑定。
fn bake_entry(
    block_id: &str,
    entry: &serde_json::Value,
    when: Vec<(String, String)>,
    raw: &RawPack,
    cache: &mut HashMap<String, ResolvedModel>,
    layer: RenderLayer,
    exclusive: bool,
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
    // `uvlock: true` 表示「方块转的时候，贴图方向不跟着转」。
    let uvlock = entry
        .get("uvlock")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);

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

    // 没有可用元素时的处理。
    //
    // **水/熔岩**：回退成整立方体。`block/water.json` 里只有 `particle`，原版
    // 靠渲染器的流体几何画它，不回退就完全没有水。判断依据照 shulkr 的 `Xt()`。
    //
    // **其余**：方块实体（告示牌、床、箱子、旗帜、潜影盒……）没有 `elements`，
    // 原版由各自的 Renderer 绘制，投影预览没有等效物。这里产出**空模型**：
    // 前端看到 `quads` 为空就当它不存在（不画），但**不计入「缺贴图」** ——
    // 「缺贴图」是材质版本对不上，混在一起会报出几百个假警报。
    if quads.is_empty() && !is_always_tinted(block_id) {
        return Some(VariantBinding {
            when,
            exclusive: true,
            rotation_x,
            rotation_y,
            model: BakedModel {
                quads: Vec::new(),
                texture_ids: Vec::new(),
                full_cube: false,
                ambient_occlusion: true,
            },
        });
    }

    if quads.is_empty() {
        let texture = resolved
            .textures
            .values()
            .filter_map(|reference| resolve_texture(reference, &resolved.textures))
            .next()?;
        let fallback_face = RawFace {
            // 回退出来的整立方体没有 `uv` 可依据，显式给整格。
            uv: Some([0.0, 0.0, 16.0, 16.0]),
            texture: "#all".to_owned(),
            cullface: None,
            // 水和 `waterlogged` 的方块永远染色；其余回退方块按普通纹理画。
            tinted: is_always_tinted(block_id),
            rotation: 0,
        };
        let cube = RawElement {
            from: [0.0; 3],
            to: [16.0; 3],
            faces: ALL_FACES
                .iter()
                .map(|face| (*face, fallback_face.clone()))
                .collect(),
            rotation: None,
            shade: true,
        };
        for face in ALL_FACES {
            let mut quad = build_quad(&cube, face, &fallback_face, layer);
            quad.uv = Some((texture.clone(), texture.clone()));
            quads.push(quad);
        }
        texture_ids.insert(texture);
    }

    let _ = uvlock;
    Some(VariantBinding {
        when,
        exclusive,
        rotation_x,
        rotation_y,
        model: BakedModel {
            quads,
            texture_ids: texture_ids.into_iter().collect(),
            full_cube: resolved.elements.len() == 1 && resolved.elements[0].is_full_cube(),
            ambient_occlusion: resolved.ambient_occlusion.unwrap_or(true),
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
            if let Some(reference) = texture_reference(entry) {
                resolved.textures.insert(key.clone(), reference);
            }
        }
    }

    // 出现 `elements` 就是整体替换，和 Minecraft 的语义一致。
    if let Some(elements) = value.get("elements").and_then(|e| e.as_array()) {
        resolved.elements = elements.iter().filter_map(parse_element).collect();
    }

    // `ambientocclusion` 子模型写了就覆盖，没写就沿用父链的值。
    if let Some(flag) = value
        .get("ambientocclusion")
        .and_then(serde_json::Value::as_bool)
    {
        resolved.ambient_occlusion = Some(flag);
    }

    /// 读取一个贴图引用。
    ///
    /// 旧格式是裸字符串 `"minecraft:block/stone"`；26.x 起可以写成对象，
    /// 用 `sprite` 指向真正的贴图，旁边还有 `force_translucent` 这类标志：
    ///
    /// ```json
    /// "all": { "force_translucent": true, "sprite": "minecraft:block/glass" }
    /// ```
    ///
    /// 只认字符串的话，这类方块会因为纹理引用丢失而整个烘焙失败 —— 玻璃就是
    /// 这么消失的，画面上表现为墙上开洞。
    fn texture_reference(entry: &serde_json::Value) -> Option<String> {
        if let Some(text) = entry.as_str() {
            return Some(text.to_owned());
        }
        entry
            .get("sprite")
            .and_then(|sprite| sprite.as_str())
            .map(str::to_owned)
    }

    cache.insert(id.to_owned(), resolved.clone());
    Some(resolved)
}

/// 一个 `elements` 条目。
fn parse_element(value: &serde_json::Value) -> Option<RawElement> {
    let from = parse_vec3(value.get("from")?)?;
    let to = parse_vec3(value.get("to")?)?;
    let rotation = parse_element_rotation(value.get("rotation"));
    let faces_object = value.get("faces")?.as_object()?;

    let mut faces = Vec::new();
    for (name, face_value) in faces_object {
        let Some(face) = parse_face_name(name) else {
            continue;
        };
        let Some(texture) = face_value.get("texture").and_then(|t| t.as_str()) else {
            continue;
        };
        // `uv` 缺省时**不能**当成 0,0,16,16 —— 原版会按面朝向从 `from`/`to`
        // 推导一张"刚好铺满这个面"的 UV。书架、告示牌这些多盒子模型全靠这个
        // 默认值，一律给整格会让每个盒子的贴图都缩放到 16×16，看起来错位。
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
            });
        let cullface = face_value
            .get("cullface")
            .and_then(|c| c.as_str())
            .and_then(parse_face_name);
        // `tintindex` 存在就表示这个面要乘群系色。值是 0/1 之类的索引，
        // 我们只区分「染/不染」：草丛和树叶本来就只有一种群系色。
        let tinted = face_value.get("tintindex").is_some();
        // 面级 UV 旋转（楼梯、门）。漏掉会让贴图整体转 90°。
        let rotation = face_value
            .get("rotation")
            .and_then(|value| value.as_i64())
            .unwrap_or(0) as i32;
        faces.push((
            face,
            RawFace {
                uv,
                texture: texture.to_owned(),
                cullface,
                tinted,
                rotation,
            },
        ));
    }

    if faces.is_empty() {
        return None;
    }
    let shade = value
        .get("shade")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(true);
    Some(RawElement {
        from,
        to,
        faces,
        rotation,
        shade,
    })
}

/// 解析 `elements[].rotation`。
fn parse_element_rotation(value: Option<&serde_json::Value>) -> Option<ElementRotation> {
    let value = value?;
    let origin = value
        .get("origin")
        .and_then(parse_vec3)
        .unwrap_or([8.0, 8.0, 8.0]);
    let axis = match value.get("axis").and_then(|a| a.as_str())? {
        "x" => RotationAxis::X,
        "y" => RotationAxis::Y,
        "z" => RotationAxis::Z,
        _ => return None,
    };
    let angle = value.get("angle").and_then(serde_json::Value::as_f64)? as f32;
    let rescale = value
        .get("rescale")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    Some(ElementRotation {
        origin,
        axis,
        angle,
        rescale,
    })
}

/// 一个面的 UV，四个顶点各一份（0..1，相对**这张贴图**）。
///
/// 逐条照抄 shulkr 的 `pt()`，包括两处容易漏的细节：
///
/// 1. **`uv` 缺省时按面朝向推导**。每个面的 u/v 取值方向都不同（`north` 用
///    `(16 - 末端)/16`，`south` 用 `起点/16`……）。一律当成整格会让书架、
///    告示牌这类多盒子模型每个盒子都拉伸到 16×16，贴图全部错位。
/// 2. **按面朝向换顶点顺序**。上/下面和侧面的排布顺序不同，这是楼梯侧面、
///    门贴图转 90° 的根源。v 轴**不**翻转，原因见函数体。
///
/// `rotation` 是面级 UV 旋转（0/90/180/270），按 90° 循环挪动顶点。
fn face_uvs(
    face: Face,
    from: [f32; 3],
    to: [f32; 3],
    explicit: Option<[f32; 4]>,
    rotation: i32,
) -> [[f32; 2]; 4] {
    // 顶点 [u0, v0, u1, v1]，单位是贴图像素（0..16）。
    let [r, i, a, o] = match explicit {
        Some([u0, v0, u1, v1]) => [u0 / 16.0, v0 / 16.0, u1 / 16.0, v1 / 16.0],
        None => {
            let [fx, fy, fz] = from;
            let [tx, ty, tz] = to;
            match face {
                Face::Down => [fx, 1.0 - tz, tx, 1.0 - fz],
                Face::Up => [fx, fz, tx, tz],
                Face::North => [1.0 - tx, 1.0 - ty, 1.0 - fx, 1.0 - fy],
                Face::South => [fx, 1.0 - ty, tx, 1.0 - fy],
                Face::West => [fz, 1.0 - ty, tz, 1.0 - fy],
                Face::East => [1.0 - tz, 1.0 - ty, 1.0 - fz, 1.0 - fy],
            }
        }
    };

    // **v 不翻转。** 图集是 `flipY = false` 上传的，纹理 v=0 就是图片第 0 行
    // （顶部），和 Minecraft 的 UV 同向。shulkr 的 `pt()` 在这里做了
    // `1 - v`，那是配合 three.js 默认 `flipY = true` 的写法；照抄过来再关掉
    // flipY 就是翻两遍：侧面全部上下颠倒（花草倒挂、灯笼链子朝下、活塞头
    // 跑到另一侧）。纯色或对称贴图看不出来，所以石头、木板一直「看起来对」。
    //
    // 顶点顺序见 `face_corners`：侧面是「右下、右上、左上、左下」，上/下面
    // 是「左下、右下、右上、左上」（以原版贴图方向为准）。
    let (bottom, top) = (o, i);
    let mut uvs: [[f32; 2]; 4] = match face {
        Face::Up | Face::Down => [[r, bottom], [a, bottom], [a, top], [r, top]],
        _ => [[a, bottom], [a, top], [r, top], [r, bottom]],
    };

    // 面级旋转：每 90° 把顶点循环挪一位。
    let steps = ((rotation / 90) % 4 + 4) % 4;
    for _ in 0..steps {
        uvs = [uvs[1], uvs[2], uvs[3], uvs[0]];
    }
    uvs
}

/// 把一个面变成四边形，UV 照原版 `ModelElement` 的规则推导。
///
/// 顶点顺序与 UV 必须配成对，否则贴图整体旋转。原版 `BakedQuad` 的四个顶点
/// 是「左上、左下、右下、右上」，`[u0, v0, u1, v1]` 是贴图上的一个矩形
/// （`v0` 在上、`v1` 在下）。
///
/// UV 的推导见 [`face_uvs`]：模型里写了 `uv` 就用它，没写就按 `from`/`to`
/// 沿面朝向算一张刚好铺满的 UV —— 书架、告示牌这类多盒子模型全靠这个。
fn build_quad(element: &RawElement, face: Face, raw_face: &RawFace, layer: RenderLayer) -> Quad {
    let from = element.from.map(|value| value / 16.0);
    let to = element.to.map(|value| value / 16.0);
    let uvs = face_uvs(face, from, to, raw_face.uv, raw_face.rotation);

    let mut positions = face_corners(face, from, to);
    let mut normal = face_normal(face);
    // 元素自带的旋转（花草的十字就是绕 Y 转 45°）。漏掉它，十字会退化成
    // 一张贴在方块中间的黑底薄片。
    if let Some(rotation) = &element.rotation {
        let radians = rotation.angle.to_radians();
        let (sin, cos) = (radians.sin(), radians.cos());
        // JSON 里 origin 是 0..16 像素，这里换成 0..1 方块空间。
        let origin = rotation.origin.map(|value| value / 16.0);
        for position in positions.iter_mut() {
            *position = rotate_about_axis(*position, origin, rotation.axis, sin, cos);
        }
        normal = rotate_direction(normal, rotation.axis, sin, cos);

        // `rescale` 让斜置的盒子重新撑满原来的外接尺寸：把被旋转压缩的那
        // 两个轴按 1/|cos| 拉伸回去。
        if rotation.rescale
            && let Some(scale) = rescale_factor(sin, cos)
        {
            for position in positions.iter_mut() {
                for axis in scale.axes {
                    position[axis] = origin[axis] + (position[axis] - origin[axis]) * scale.factor;
                }
            }
        }
    }

    Quad {
        positions,
        uvs,
        normal,
        cullface: raw_face.cullface,
        layer,
        uv: None,
        // `tintindex` 只决定「这个面要染色」；具体颜色在 `assign_tints` 里按
        // 方块 id 补上 —— 那里才拿得到方块名（树叶和草的颜色不同）。
        // 这里先用 `[0,0,0]` 占位表示「待染色」，全 1 表示不染。
        tint: if raw_face.tinted {
            [0.0, 0.0, 0.0]
        } else {
            [1.0, 1.0, 1.0]
        },
        shade: element.shade,
    }
}

/// 绕坐标轴旋转一个点（先平移到 `origin`，转完再平移回去）。
fn rotate_about_axis(
    point: [f32; 3],
    origin: [f32; 3],
    axis: RotationAxis,
    sin: f32,
    cos: f32,
) -> [f32; 3] {
    let local = [
        point[0] - origin[0],
        point[1] - origin[1],
        point[2] - origin[2],
    ];
    let rotated = rotate_direction(local, axis, sin, cos);
    [
        rotated[0] + origin[0],
        rotated[1] + origin[1],
        rotated[2] + origin[2],
    ]
}

/// 只旋转方向（不平移），用于法线。
fn rotate_direction(vector: [f32; 3], axis: RotationAxis, sin: f32, cos: f32) -> [f32; 3] {
    match axis {
        RotationAxis::X => [
            vector[0],
            vector[1] * cos - vector[2] * sin,
            vector[1] * sin + vector[2] * cos,
        ],
        RotationAxis::Y => [
            vector[0] * cos + vector[2] * sin,
            vector[1],
            -vector[0] * sin + vector[2] * cos,
        ],
        RotationAxis::Z => [
            vector[0] * cos - vector[1] * sin,
            vector[0] * sin + vector[1] * cos,
            vector[2],
        ],
    }
}

/// `rescale` 需要的拉伸：被旋转的两个轴按 `1/|cos|` 放大。
fn rescale_factor(sin: f32, cos: f32) -> Option<Rescale> {
    if sin.abs() < 1e-4 || cos.abs() < 1e-4 {
        // 45° 之外的角度拉伸不可逆，原版也只对 22.5/45 生效。
        return None;
    }
    let factor = 1.0 / cos.abs();
    if (factor - 1.0).abs() < 1e-4 {
        return None;
    }
    Some(Rescale {
        axes: [0, 2],
        factor,
    })
}

struct Rescale {
    axes: [usize; 2],
    factor: f32,
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
/// 六个面的四个角，顺序与 [`build_quad`] 里的 UV 一一对应。
///
/// Minecraft 的坐标：`-Z` 是北，`+X` 是东，`+Y` 是上。每个面的第 0 个顶点是
/// 从外面看时视觉上的左上角 —— 这是原版烘焙器自己的约定，换个顺序贴图就会
/// 转 90° 或镜像。
fn face_corners(face: Face, from: [f32; 3], to: [f32; 3]) -> [[f32; 3]; 4] {
    // 模型里允许 `from` 大于 `to`（原版有些「内表面」就是这么写的，比如
    // `cube_bottom_top_inner_faces` 的第二个盒子）。先归一化成 min/max，
    // 否则角点的左右会颠倒，绕序和贴图一起反掉。
    let x0 = from[0].min(to[0]);
    let y0 = from[1].min(to[1]);
    let z0 = from[2].min(to[2]);
    let x1 = from[0].max(to[0]);
    let y1 = from[1].max(to[1]);
    let z1 = from[2].max(to[2]);
    // **顺序必须和 shulkr 的 `ft()` 逐字一致**：UV 是按这个顺序排的，
    // 换了顺序贴图就会整体错位（看起来像"侧面没了"）。
    match face {
        Face::Down => [[x0, y0, z0], [x1, y0, z0], [x1, y0, z1], [x0, y0, z1]],
        Face::Up => [[x0, y1, z1], [x1, y1, z1], [x1, y1, z0], [x0, y1, z0]],
        Face::North => [[x0, y0, z0], [x0, y1, z0], [x1, y1, z0], [x1, y0, z0]],
        Face::South => [[x1, y0, z1], [x1, y1, z1], [x0, y1, z1], [x0, y0, z1]],
        Face::West => [[x0, y0, z1], [x0, y1, z1], [x0, y1, z0], [x0, y0, z0]],
        Face::East => [[x1, y0, z0], [x1, y1, z0], [x1, y1, z1], [x1, y0, z1]],
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

/// 解析 multipart 里的 `when` 对象成「必须同时满足」的属性列表。
///
/// 支持三种写法（照 shulkr 的 `ze()`）：
///
/// - 平铺键值：`{"facing":"north"}`；
/// - 多选值：`{"facing":"north|east"}` —— 由前端按 `|` 拆分判断；
/// - 逻辑组合：`{"AND":[{...},{...}]}` / `{"OR":[...]}`。
///
/// `cherry_shelf` 这类方块全靠 `AND` 区分朝向和连接状态。之前把 `AND` 当成
/// 普通键、只取数组第一个元素，结果整条条件被丢掉 —— 十几个变体全部无条件
/// 命中、叠在一起，画面上就是一团乱。
///
/// 绑定结构只表达 AND，所以 `OR` 取第一个分支（近似，但比全画强）；
/// `AND` 里的每个子条件递归展开后追加。
fn parse_when_condition(value: &serde_json::Value) -> Vec<(String, String)> {
    let Some(object) = value.as_object() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (name, raw) in object {
        match name.as_str() {
            "AND" => {
                if let Some(parts) = raw.as_array() {
                    for part in parts {
                        out.extend(parse_when_condition(part));
                    }
                }
                continue;
            }
            "OR" => {
                // 只表达 AND 的结构装不下「任一」，取第一个分支。
                if let Some(first) = raw.as_array().and_then(|parts| parts.first()) {
                    out.extend(parse_when_condition(first));
                }
                continue;
            }
            _ => {}
        }
        let scalar = match raw {
            serde_json::Value::String(text) => Some(text.clone()),
            serde_json::Value::Bool(flag) => Some(flag.to_string()),
            serde_json::Value::Number(number) => Some(number.to_string()),
            // 栅栏的 `"north": [{"facing":"north","value":true}]` 这种数组：取第一个。
            serde_json::Value::Array(branches) => branches.first().and_then(|branch| {
                branch.get("value").and_then(|wanted| match wanted {
                    serde_json::Value::String(text) => Some(text.clone()),
                    serde_json::Value::Bool(flag) => Some(flag.to_string()),
                    _ => None,
                })
            }),
            _ => None,
        };
        if let Some(scalar) = scalar {
            out.push((name.clone(), scalar));
        }
    }
    out
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

/// 按贴图的 alpha 修正每个面的渲染层。
///
/// 判据完全来自贴图像素（见 `pack::build_atlas` 的 `transparent` / `blended`），
/// 不维护方块名名单：
///
/// - 贴图不透明 → `opaque`，不混合，最快；
/// - 有 `0 < alpha < 255` 的像素 → `translucent`，真正按像素混合。彩色玻璃、
///   冰、水属于这一类；
/// - 只有 `alpha == 0` 和 `alpha == 255`（二值）→ `cutout`，用 `alphaTest`
///   裁掉全透明像素。普通玻璃、草、花、树叶属于这一类。
///
/// 之前是按方块名猜的（只认 `glass`），结果 `pink_stained_glass` 落进 cutout
/// 变成硬边、`glass` 被当成半透明整体提亮 —— 两种都不对。
pub(super) fn assign_layers_from_textures(
    blocks: &mut HashMap<String, Vec<VariantBinding>>,
    transparent: &std::collections::HashSet<String>,
    blended: &std::collections::HashSet<String>,
) {
    for bindings in blocks.values_mut() {
        for binding in bindings.iter_mut() {
            for quad in &mut binding.model.quads {
                let Some((texture, _)) = &quad.uv else {
                    continue;
                };
                quad.layer = if blended.contains(texture) {
                    RenderLayer::Translucent
                } else if transparent.contains(texture) {
                    RenderLayer::Cutout
                } else {
                    RenderLayer::Opaque
                };
            }
        }
    }
}

/// 给待染色的面补上颜色。
///
/// `build_quad` 阶段只标了「这个面要染色」（占位 `[0,0,0]`），因为它拿不到
/// 方块 id。这里遍历按方块分组的数据，按 id 决定用草色、树叶色还是水色。
pub(super) fn assign_tints(blocks: &mut HashMap<String, Vec<VariantBinding>>) {
    for (block_id, bindings) in blocks.iter_mut() {
        let tint = tint_for_block(block_id);
        for binding in bindings.iter_mut() {
            for quad in &mut binding.model.quads {
                // 只有占位值才替换，`[1,1,1]` 是「本来就不染」。
                if quad.tint == [0.0, 0.0, 0.0] {
                    quad.tint = tint;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn element(from: [f32; 3], to: [f32; 3], shade: bool) -> RawElement {
        RawElement {
            from,
            to,
            faces: Vec::new(),
            rotation: None,
            shade,
        }
    }

    fn face(uv: Option<[f32; 4]>) -> RawFace {
        RawFace {
            uv,
            texture: "#all".to_owned(),
            cullface: None,
            tinted: false,
            rotation: 0,
        }
    }

    /// 侧面靠下的顶点必须取贴图靠下的像素（v 更大）。
    ///
    /// 图集是 `flipY = false` 上传的，v 越大越靠图片底部。v 被翻过的话，花的
    /// 花朵会落到茎下面、灯笼倒挂。
    #[test]
    fn side_faces_keep_the_texture_upright() {
        let cube = element([0.0; 3], [16.0; 3], true);
        for side in [Face::North, Face::South, Face::West, Face::East] {
            let quad = build_quad(
                &cube,
                side,
                &face(Some([0.0, 0.0, 16.0, 16.0])),
                RenderLayer::Opaque,
            );
            for (position, uv) in quad.positions.iter().zip(quad.uvs.iter()) {
                let expected = if position[1] < 0.5 { 1.0 } else { 0.0 };
                assert_eq!(
                    uv[1], expected,
                    "{side:?}: vertex {position:?} got uv {uv:?}"
                );
            }
        }
    }

    /// 没写 `uv` 的侧面按 `from`/`to` 推导：下半格的盒子取贴图下半部分。
    #[test]
    fn derived_side_uv_follows_the_element_height() {
        let lower_half = element([0.0; 3], [16.0, 8.0, 16.0], true);
        let quad = build_quad(&lower_half, Face::North, &face(None), RenderLayer::Opaque);
        for (position, uv) in quad.positions.iter().zip(quad.uvs.iter()) {
            let expected = if position[1] < 0.25 { 1.0 } else { 0.5 };
            assert_eq!(uv[1], expected, "vertex {position:?} got uv {uv:?}");
        }
    }

    /// 顶面朝北的一边对贴图顶部（v=0），和原版一致。
    #[test]
    fn the_top_face_has_north_at_the_texture_top() {
        let cube = element([0.0; 3], [16.0; 3], true);
        let quad = build_quad(
            &cube,
            Face::Up,
            &face(Some([0.0, 0.0, 16.0, 16.0])),
            RenderLayer::Opaque,
        );
        for (position, uv) in quad.positions.iter().zip(quad.uvs.iter()) {
            let expected = if position[2] < 0.5 { 0.0 } else { 1.0 };
            assert_eq!(uv[1], expected, "vertex {position:?} got uv {uv:?}");
        }
    }

    #[test]
    fn element_shade_reaches_the_quad() {
        let flat = element([0.0; 3], [16.0; 3], false);
        let quad = build_quad(&flat, Face::Up, &face(None), RenderLayer::Cutout);
        assert!(!quad.shade);
    }

    /// `ambientocclusion` 沿父链继承，子模型写了才覆盖。
    #[test]
    fn ambient_occlusion_is_inherited_from_the_parent() {
        let mut raw = RawPack::default();
        raw.models.insert(
            "minecraft:block/cross".to_owned(),
            r#"{"ambientocclusion": false}"#.to_owned(),
        );
        raw.models.insert(
            "minecraft:block/dandelion".to_owned(),
            r#"{"parent": "minecraft:block/cross"}"#.to_owned(),
        );
        raw.models
            .insert("minecraft:block/cube".to_owned(), r#"{}"#.to_owned());
        let mut cache = HashMap::new();
        let flower =
            resolve_model("minecraft:block/dandelion", &raw, &mut cache, 0).expect("flower");
        assert_eq!(flower.ambient_occlusion, Some(false));
        let cube = resolve_model("minecraft:block/cube", &raw, &mut cache, 0).expect("cube");
        assert_eq!(cube.ambient_occlusion, None);
    }
}
