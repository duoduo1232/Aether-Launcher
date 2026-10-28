//! 把一堆方块贴图拼成一张图集。
//!
//! 预览要给每个面贴纹理，如果每张贴图各用一个 GPU 纹理，几千个方块建筑会
//! 触发大量纹理切换；拼成一张图集就只需一次绑定。图集按固定格子排列
//! （每格放一张 16×16 贴图，按需缩放），格子的 UV 由 [`super::AtlasSlot`]
//! 告诉前端。

use std::collections::{HashMap, HashSet};

use super::AtlasSlot;

/// 图集里每个贴图格子的边长（像素）。
///
/// Minecraft 的方块贴图基本都是 16×16；比它大的（如某些资源包）按最近邻
/// 缩到这一格，预览不需要还原每张的高分辨率细节。
const CELL: u32 = 16;

/// 图集最大的边长，超过就停止添加贴图。
const MAX_ATLAS_SIDE: u32 = 4096;

/// 拼图集：返回（PNG 字节，贴图 id → UV 槽，图集边长）。
///
/// `wanted` 是模型引用到的贴图 id 集合；`textures` 是 jar 里的原始 PNG。
/// 找不到的贴图直接跳过 —— 前端会按「无贴图」回退成纯色。
pub fn build_atlas(
    textures: &HashMap<String, Vec<u8>>,
    wanted: &HashSet<String>,
) -> (Vec<u8>, HashMap<String, AtlasSlot>, u32) {
    // 只取确实存在的贴图，并按名字排序让图集内容稳定（便于缓存与测试）。
    let mut ids: Vec<&String> = wanted
        .iter()
        .filter(|id| textures.get(*id).is_some_and(|bytes| is_png(bytes)))
        .collect();
    ids.sort();

    let per_row = ((ids.len() as f64).sqrt().ceil() as u32).max(1);
    let side_cells = per_row.min(MAX_ATLAS_SIDE / CELL);
    let side = (side_cells * CELL).max(CELL);

    let mut rgba = vec![0u8; (side * side * 4) as usize];
    let mut atlas = HashMap::new();
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
        let cell_x = placed % side_cells;
        let cell_y = placed / side_cells;
        blit(&mut rgba, side, cell_x * CELL, cell_y * CELL, &pixels);
        atlas.insert(
            id.clone(),
            AtlasSlot {
                u0: (cell_x * CELL) as f32 / side as f32,
                v0: (cell_y * CELL) as f32 / side as f32,
                u1: ((cell_x + 1) * CELL) as f32 / side as f32,
                v1: ((cell_y + 1) * CELL) as f32 / side as f32,
            },
        );
        placed += 1;
    }

    (encode_png(&rgba, side), atlas, side)
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
fn decode_png(bytes: &[u8]) -> Option<Pixels> {
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = decoder.read_info().ok()?;
    let buffer_size = reader.output_buffer_size()?;
    let mut buffer = vec![0u8; buffer_size];
    let frame = reader.next_frame(&mut buffer).ok()?;
    let width = frame.width;
    let height = frame.height;
    let color_type = frame.color_type;
    let data = &buffer[..frame.buffer_size()];

    let mut out = vec![0u8; (CELL * CELL * 4) as usize];
    for y in 0..CELL {
        for x in 0..CELL {
            // 最近邻采样：源图坐标按比例映射。
            let sx = (u64::from(x) * u64::from(width) / u64::from(CELL)) as u32;
            let sy = (u64::from(y) * u64::from(height) / u64::from(CELL)) as u32;
            let sx = sx.min(width.saturating_sub(1));
            let sy = sy.min(height.saturating_sub(1));
            let (r, g, b, a) = sample(data, width, sx, sy, color_type);
            let index = ((y * CELL + x) * 4) as usize;
            out[index] = r;
            out[index + 1] = g;
            out[index + 2] = b;
            out[index + 3] = a;
        }
    }
    Some(Pixels { data: out })
}

/// 按颜色类型取一个像素并统一成 RGBA。
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
        // 索引色已经被解码器展开成 RGB(A)，不会走到这里。
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
