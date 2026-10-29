//! 把一堆方块贴图拼成一张图集。
//!
//! 预览要给每个面贴纹理，如果每张贴图各用一个 GPU 纹理，几千个方块建筑会
//! 触发大量纹理切换；拼成一张图集就只需一次绑定。图集按固定格子排列
//! （每格放一张 16×16 贴图，按需缩放），格子的 UV 由 [`super::AtlasSlot`]
//! 告诉前端。

use std::collections::{HashMap, HashSet};

use super::AtlasSlot;

/// 图集里每张贴图的**内容**边长（像素）。
///
/// Minecraft 的方块贴图基本都是 16×16；比它大的（如某些资源包）按最近邻
/// 缩到这一格，预览不需要还原每张的高分辨率细节。
const CELL: u32 = 16;

/// 每张贴图周围的留白（像素），用边缘像素填充。
///
/// 不留白的话，相邻两张贴图直接接在一起，UV 在格子边界采样时会混入邻居的
/// 颜色 —— 画面上就是每个方块四周一圈黑边（玻璃尤其明显）。留 2 像素并用
/// `edge` 方式复制边缘，越界采样拿到的仍是自己的颜色。
const PADDING: u32 = 2;

/// 图集最大的边长，超过就停止添加贴图。
const MAX_ATLAS_SIDE: u32 = 4096;

/// 拼图集：返回（PNG 字节，贴图 id → UV 槽，图集边长）。
///
/// `wanted` 是模型引用到的贴图 id 集合；`textures` 是 jar 里的原始 PNG。
/// 找不到的贴图直接跳过 —— 前端会按「无贴图」回退成纯色。
pub fn build_atlas(textures: &HashMap<String, Vec<u8>>, wanted: &HashSet<String>) -> AtlasBuild {
    // 只取确实存在的贴图，并按名字排序让图集内容稳定（便于缓存与测试）。
    let mut ids: Vec<&String> = wanted
        .iter()
        .filter(|id| textures.get(*id).is_some_and(|bytes| is_png(bytes)))
        .collect();
    ids.sort();
    // 每格实际占 `CELL + 2*PADDING` 像素，中间 PADDING 是复制的边缘。
    let cell_stride = CELL + PADDING * 2;
    let per_row = ((ids.len() as f64).sqrt().ceil() as u32).max(1);
    let side_cells = per_row.min(MAX_ATLAS_SIDE / cell_stride);
    let side = (side_cells * cell_stride).max(cell_stride);

    let mut rgba = vec![0u8; (side * side * 4) as usize];
    let mut atlas = HashMap::new();
    let mut transparent = HashSet::new();
    let mut blended = HashSet::new();
    let mut placed = 0u32;

    for id in ids {
        if placed >= side_cells * side_cells {
            break;
        }
        let Some(bytes) = textures.get(id) else {
            continue;
        };
        let Some(pixels) = decode_png(bytes) else {
            continue;
        };
        // 按**像素**判断这张贴图需要哪一种透明处理，不维护方块名名单：
        //
        // - 出现 `alpha == 0`（完全透明）：必须走 cutout，用 `alphaTest` 把
        //   透明像素丢掉。否则背景会当不透明画出来 —— 花草「黑底」就是这么来的。
        // - 出现 `0 < alpha < 255`（半透明）：必须走 translucent，真正按像素
        //   混合，否则彩色玻璃、冰、水会变成硬边。
        //
        // 一张贴图可以两者都有（比如玻璃边框不透明、中间全透），这时以
        // **半透明**为准 —— 混合层同样能画对全透明像素。
        let mut has_zero = false;
        let mut has_partial = false;
        for pixel in pixels.data.as_chunks::<4>().0 {
            match pixel[3] {
                0 => has_zero = true,
                255 => {}
                _ => has_partial = true,
            }
            if has_zero && has_partial {
                break;
            }
        }
        if has_zero {
            transparent.insert(id.clone());
        }
        if has_partial {
            blended.insert(id.clone());
        }
        let cell_x = placed % side_cells;
        let cell_y = placed / side_cells;
        let origin_x = cell_x * cell_stride + PADDING;
        let origin_y = cell_y * cell_stride + PADDING;
        blit(&mut rgba, side, origin_x, origin_y, &pixels);
        // 四周填充边缘像素，UV 越界时不会串到邻居。
        pad_edges(&mut rgba, side, origin_x, origin_y);
        // UV 指向**内容**区域，不含留白。
        atlas.insert(
            id.clone(),
            AtlasSlot {
                u0: origin_x as f32 / side as f32,
                // `flipY = false`，纹理 v=0 对应图片第 0 行（顶部），而
                // `origin_y` 也是从图片顶部算的 —— 两边同向，**不需要翻**。
                // 之前翻过一次，结果是每个方块都取到图集里另一行的贴图。
                v0: origin_y as f32 / side as f32,
                u1: (origin_x + CELL) as f32 / side as f32,
                v1: (origin_y + CELL) as f32 / side as f32,
            },
        );
        placed += 1;
    }

    AtlasBuild {
        png: encode_png(&rgba, side),
        slots: atlas,
        side,
        transparent,
        blended,
    }
}

/// 图集构建结果。
pub struct AtlasBuild {
    pub png: Vec<u8>,
    pub slots: HashMap<String, AtlasSlot>,
    pub side: u32,
    /// 出现 `alpha == 0` 的贴图。这些方块要走 cutout（`alphaTest` 裁掉全透明）。
    pub transparent: HashSet<String>,
    /// 出现 `0 < alpha < 255` 的贴图。这些方块要走 translucent（真正按像素混合）。
    pub blended: HashSet<String>,
}

/// 最小的 PNG 合法性检查：签名 + IHDR 存在。
fn is_png(bytes: &[u8]) -> bool {
    bytes.len() > 8 && bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a])
}

/// 解码后的方形贴图（统一缩放到 [`CELL`]×[`CELL`]）。
struct Pixels {
    data: Vec<u8>,
}

/// 解码 PNG 并缩放到固定格子。
///
/// 交给解码器做颜色转换，不自己按 `ColorType` 逐像素取字节 —— 原版贴图里有大量
/// **索引色** PNG，手写分支拿不到调色板，只能吐占位色，画面上就是一片洋红。
fn decode_png(bytes: &[u8]) -> Option<Pixels> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    // 索引色展开成 RGB(A)、低位深补齐到 8 位。
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().ok()?;
    let buffer_size = reader.output_buffer_size()?;
    let mut buffer = vec![0u8; buffer_size];
    let frame = reader.next_frame(&mut buffer).ok()?;
    let width = frame.width;
    let height = frame.height;
    if width == 0 || height == 0 {
        return None;
    }
    let data = &buffer[..frame.buffer_size()];
    let source_height = first_frame_height(width, height);

    let mut out = vec![0u8; (CELL * CELL * 4) as usize];
    for y in 0..CELL {
        for x in 0..CELL {
            // 最近邻采样：源图坐标按比例映射。
            let sx = ((u64::from(x) * u64::from(width)) / u64::from(CELL)) as u32;
            let sy = ((u64::from(y) * u64::from(source_height)) / u64::from(CELL)) as u32;
            let sx = sx.min(width - 1);
            let sy = sy.min(source_height - 1);
            let (r, g, b, a) = sample(data, width, sx, sy, frame.color_type);
            let index = ((y * CELL + x) * 4) as usize;
            out[index] = r;
            out[index + 1] = g;
            out[index + 2] = b;
            out[index + 3] = a;
        }
    }
    Some(Pixels { data: out })
}

/// 要采样的源图高度：动画贴图只取第一帧。
///
/// 带 `.mcmeta` 的动画贴图（灯笼、海晶灯、岩浆块……）是一条竖向长图，
/// 每帧 `width × width` 上下叠放。把整条压进一个格子会让几帧隔行混在一起、
/// 纵向被挤扁，模型里精确到像素的 UV 就全部取错。预览不播动画，取第一帧。
fn first_frame_height(width: u32, height: u32) -> u32 {
    if height > width && height.is_multiple_of(width) {
        width
    } else {
        height
    }
}

/// 按颜色类型取一个像素并统一成 RGBA。
///
/// 索引色已经由 `normalize_to_color8` 展开，正常不会走到 `Indexed` 分支。
fn sample(data: &[u8], width: u32, x: u32, y: u32, color: png::ColorType) -> (u8, u8, u8, u8) {
    let channels = match color {
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::Indexed => 1,
    };
    let index = ((y * width + x) * channels) as usize;
    match color {
        png::ColorType::Grayscale => {
            let value = data.get(index).copied().unwrap_or(0);
            (value, value, value, 255)
        }
        png::ColorType::GrayscaleAlpha => {
            let value = data.get(index).copied().unwrap_or(0);
            let alpha = data.get(index + 1).copied().unwrap_or(255);
            (value, value, value, alpha)
        }
        png::ColorType::Rgb => (
            data.get(index).copied().unwrap_or(0),
            data.get(index + 1).copied().unwrap_or(0),
            data.get(index + 2).copied().unwrap_or(0),
            255,
        ),
        png::ColorType::Rgba => (
            data.get(index).copied().unwrap_or(0),
            data.get(index + 1).copied().unwrap_or(0),
            data.get(index + 2).copied().unwrap_or(0),
            data.get(index + 3).copied().unwrap_or(255),
        ),
        // 展开前的兜底：明显的洋红，方便发现「转换没生效」。
        png::ColorType::Indexed => (255, 0, 255, 255),
    }
}

/// 把一格像素贴到图集上。
fn blit(target: &mut [u8], side: u32, x0: u32, y0: u32, source: &Pixels) {
    for y in 0..CELL {
        for x in 0..CELL {
            let src = ((y * CELL + x) * 4) as usize;
            let dst = (((y0 + y) * side + (x0 + x)) * 4) as usize;
            if let (Some(from), Some(to)) =
                (source.data.get(src..src + 4), target.get_mut(dst..dst + 4))
            {
                to.copy_from_slice(from);
            }
        }
    }
}

/// 把 `blit` 写进去的贴图四周再刷一圈边缘像素。
///
/// 图集里相邻两格紧挨着，UV 在格子边界采样（尤其是 `NearestFilter` 关掉
/// mipmap 后仍有半像素误差）会取到邻居的颜色，画面上是每个方块一圈黑边。
/// 复制边缘像素之后，越界采样拿到的还是自己的颜色。
fn pad_edges(target: &mut [u8], side: u32, x0: u32, y0: u32) {
    let copy = |target: &mut [u8], from_x: u32, from_y: u32, to_x: u32, to_y: u32| {
        let from = ((from_y * side + from_x) * 4) as usize;
        let to = ((to_y * side + to_x) * 4) as usize;
        let pixel = [
            target[from],
            target[from + 1],
            target[from + 2],
            target[from + 3],
        ];
        target[to..to + 4].copy_from_slice(&pixel);
    };

    for pad in 1..=PADDING {
        // 上下两行：整行复制相邻行的像素。
        for x in 0..CELL {
            copy(target, x0 + x, y0, x0 + x, y0 - pad);
            copy(target, x0 + x, y0 + CELL - 1, x0 + x, y0 + CELL - 1 + pad);
        }
        // 左右两列（含刚补出来的角）。
        for y in 0..CELL + pad * 2 {
            let row = y0 + y - pad;
            if row >= side {
                continue;
            }
            copy(target, x0, row, x0 - pad, row);
            copy(target, x0 + CELL - 1, row, x0 + CELL - 1 + pad, row);
        }
    }
}

/// 把 RGBA 编码成 PNG。
fn encode_png(rgba: &[u8], side: u32) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, side, side);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        if let Ok(mut writer) = encoder.write_header() {
            let _ = writer.write_image_data(rgba);
        }
    }
    out
}

/// 测试用：解码一张 PNG，拿到 RGBA 像素。
#[cfg(test)]
pub(super) fn decode_for_test(bytes: &[u8]) -> Option<Vec<u8>> {
    decode_png(bytes).map(|pixels| pixels.data)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 编一张 `width × height` 的 RGBA PNG，像素由 `pixel(x, y)` 给出。
    fn png_of(width: u32, height: u32, pixel: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
        let mut rgba = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            for x in 0..width {
                rgba.extend_from_slice(&pixel(x, y));
            }
        }
        let mut out = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut out, width, height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().expect("header");
            writer.write_image_data(&rgba).expect("data");
        }
        out
    }

    /// 灯笼这类动画贴图是 16×48 的三帧竖条，格子里必须是完整的第一帧，
    /// 而不是三帧隔行抽样、纵向挤扁的混合。
    #[test]
    fn an_animated_strip_keeps_only_its_first_frame() {
        // 每一行的红色通道写上它的行号，帧号写进绿色通道。
        let strip = png_of(16, 48, |_, y| [y as u8, (y / 16) as u8, 0, 255]);
        let pixels = decode_for_test(&strip).expect("decode");
        for y in 0..CELL {
            let index = ((y * CELL) * 4) as usize;
            assert_eq!(
                pixels[index], y as u8,
                "row {y} sampled the wrong source row"
            );
            assert_eq!(pixels[index + 1], 0, "row {y} sampled a later frame");
        }
    }

    /// 普通的方形高清贴图仍然整张缩进格子。
    #[test]
    fn a_square_texture_is_scaled_whole() {
        let large = png_of(32, 32, |_, y| [y as u8, 0, 0, 255]);
        let pixels = decode_for_test(&large).expect("decode");
        let last_row = (((CELL - 1) * CELL) * 4) as usize;
        assert_eq!(
            pixels[last_row], 30,
            "the bottom of a 32px texture was dropped"
        );
    }
}
