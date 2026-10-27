//! 极简 NBT（Named Binary Tag）读取器。
//!
//! 只覆盖读取 `.litematic` 所需的部分：全部 13 种标签的解码与 gzip 解压入口。
//! 不做任何写入。NBT 的数值与长度前缀一律是**大端**字节序。
//!
//! 防御边界：gzip 解压有输出上限（防解压炸弹）、复合结构有深度上限、
//! 字符串必须是合法 UTF-8。输入被视为不可信数据。

use std::io::Read;

use thiserror::Error;

/// 解压输出的上限：512 MiB。
const MAX_DECOMPRESSED_BYTES: u64 = 512 * 1024 * 1024;

/// 复合/列表嵌套深度上限。
const MAX_DEPTH: usize = 64;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum NbtError {
    #[error("NBT payload ended unexpectedly")]
    UnexpectedEof,
    #[error("unknown NBT tag type {0}")]
    UnknownType(u8),
    #[error("NBT nesting exceeds the depth limit of {0}")]
    DepthLimit(usize),
    #[error("NBT string is not valid UTF-8")]
    InvalidUtf8,
    #[error("NBT root must be a named compound")]
    RootNotNamedCompound,
    #[error("gzip payload is not valid NBT container")]
    InvalidGzip,
    #[error("decompressed NBT exceeds the {MAX_DECOMPRESSED_BYTES} byte limit")]
    DecompressionLimit,
    #[error("NBT list length is negative")]
    NegativeLength,
}

/// 解析后的 NBT 值。复合结构保留原始字段顺序，重复名字的后者覆盖前者
/// （litematic 不会出现重复字段，此处按「最后一个生效」处理）。
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    ByteArray(Vec<u8>),
    String(String),
    List(Vec<Value>),
    Compound(Vec<(String, Value)>),
    IntArray(Vec<i32>),
    LongArray(Vec<i64>),
}

impl Value {
    /// 按名字在复合结构里查找字段。
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Value> {
        match self {
            Value::Compound(entries) => entries
                .iter()
                .rev()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_int(&self) -> Option<i32> {
        match self {
            Value::Int(value) => Some(*value),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_long(&self) -> Option<i64> {
        match self {
            Value::Long(value) => Some(*value),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(value) => Some(value),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_compound(&self) -> Option<&[(String, Value)]> {
        match self {
            Value::Compound(entries) => Some(entries),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_list(&self) -> Option<&[Value]> {
        match self {
            Value::List(values) => Some(values),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_long_array(&self) -> Option<&[i64]> {
        match self {
            Value::LongArray(values) => Some(values),
            _ => None,
        }
    }
}

struct Reader<'a> {
    data: &'a [u8],
    position: usize,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, position: 0 }
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8], NbtError> {
        let end = self
            .position
            .checked_add(count)
            .filter(|end| *end <= self.data.len())
            .ok_or(NbtError::UnexpectedEof)?;
        let slice = &self.data[self.position..end];
        self.position = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, NbtError> {
        Ok(self.take(1)?[0])
    }

    fn be_u16(&mut self) -> Result<u16, NbtError> {
        let bytes = self.take(2)?;
        Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
    }

    fn be_i32(&mut self) -> Result<i32, NbtError> {
        let bytes = self.take(4)?;
        Ok(i32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn be_i64(&mut self) -> Result<i64, NbtError> {
        let bytes = self.take(8)?;
        Ok(i64::from_be_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]))
    }

    fn be_f32(&mut self) -> Result<f32, NbtError> {
        Ok(f32::from_bits(self.be_i32()? as u32))
    }

    fn be_f64(&mut self) -> Result<f64, NbtError> {
        Ok(f64::from_bits(self.be_i64()? as u64))
    }

    fn string(&mut self) -> Result<String, NbtError> {
        let length = self.be_u16()? as usize;
        let bytes = self.take(length)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| NbtError::InvalidUtf8)
    }

    /// 读取一个具名标签载荷；`tag` 是外层已经读过的类型字节。
    fn payload(&mut self, tag: u8, depth: usize) -> Result<Value, NbtError> {
        if depth > MAX_DEPTH {
            return Err(NbtError::DepthLimit(MAX_DEPTH));
        }
        Ok(match tag {
            1 => Value::Byte(self.u8()? as i8),
            2 => Value::Short(self.be_u16()? as i16),
            3 => Value::Int(self.be_i32()?),
            4 => Value::Long(self.be_i64()?),
            5 => Value::Float(self.be_f32()?),
            6 => Value::Double(self.be_f64()?),
            7 => {
                let length = self.be_i32()?;
                let length = usize::try_from(length).map_err(|_| NbtError::NegativeLength)?;
                Value::ByteArray(self.take(length)?.to_vec())
            }
            8 => Value::String(self.string()?),
            9 => {
                let item_tag = self.u8()?;
                let length = self.be_i32()?;
                if length < 0 {
                    return Err(NbtError::NegativeLength);
                }
                if item_tag == 0 && length > 0 {
                    return Err(NbtError::UnknownType(0));
                }
                let mut items = Vec::new();
                if item_tag != 0 {
                    for _ in 0..length {
                        items.push(self.payload(item_tag, depth + 1)?);
                    }
                }
                Value::List(items)
            }
            10 => {
                let mut entries = Vec::new();
                loop {
                    let child = self.u8()?;
                    if child == 0 {
                        break;
                    }
                    let name = self.string()?;
                    let value = self.payload(child, depth + 1)?;
                    entries.push((name, value));
                }
                Value::Compound(entries)
            }
            11 => {
                let length = self.be_i32()?;
                let length = usize::try_from(length).map_err(|_| NbtError::NegativeLength)?;
                let mut values = Vec::with_capacity(length.min(1 << 20));
                for _ in 0..length {
                    values.push(self.be_i32()?);
                }
                Value::IntArray(values)
            }
            12 => {
                let length = self.be_i32()?;
                let length = usize::try_from(length).map_err(|_| NbtError::NegativeLength)?;
                let mut values = Vec::with_capacity(length.min(1 << 20));
                for _ in 0..length {
                    values.push(self.be_i64()?);
                }
                Value::LongArray(values)
            }
            other => return Err(NbtError::UnknownType(other)),
        })
    }
}

/// 解析未压缩的 NBT，返回根复合的名字与内容。
///
/// # Errors
/// 根不是具名复合、字节流截断、嵌套超限或类型未知时返回 [`NbtError`]。
pub fn read_nbt(bytes: &[u8]) -> Result<(String, Value), NbtError> {
    let mut reader = Reader::new(bytes);
    let tag = reader.u8()?;
    if tag != 10 {
        return Err(NbtError::RootNotNamedCompound);
    }
    let name = reader.string()?;
    let value = reader.payload(tag, 0)?;
    Ok((name, value))
}

/// 先校验 gzip 魔数，解压（有输出上限）后再按 [`read_nbt`] 解析。
///
/// # Errors
/// 输入不是 gzip、解压超限或 NBT 解析失败时返回 [`NbtError`]。
pub fn read_gzip_nbt(bytes: &[u8]) -> Result<(String, Value), NbtError> {
    if bytes.len() < 2 || bytes[0] != 0x1f || bytes[1] != 0x8b {
        return Err(NbtError::InvalidGzip);
    }
    let decoder = flate2::read::GzDecoder::new(bytes);
    let mut decompressed = Vec::new();
    decoder
        .take(MAX_DECOMPRESSED_BYTES + 1)
        .read_to_end(&mut decompressed)
        .map_err(|_| NbtError::InvalidGzip)?;
    if decompressed.len() as u64 > MAX_DECOMPRESSED_BYTES {
        return Err(NbtError::DecompressionLimit);
    }
    read_nbt(&decompressed)
}

#[cfg(test)]
#[path = "nbt_tests.rs"]
mod tests;
