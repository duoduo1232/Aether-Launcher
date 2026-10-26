use super::{NbtError, Value, read_gzip_nbt, read_nbt};

// ---------------------------------------------------------------------------
// 测试专用：手工构造 NBT 字节流
// ---------------------------------------------------------------------------

fn push_string(out: &mut Vec<u8>, value: &str) {
    out.extend_from_slice(&(value.len() as u16).to_be_bytes());
    out.extend_from_slice(value.as_bytes());
}

fn push_named(out: &mut Vec<u8>, tag: u8, name: &str, payload: &[u8]) {
    out.push(tag);
    push_string(out, name);
    out.extend_from_slice(payload);
}

fn root_with(entries: &[u8]) -> Vec<u8> {
    let mut out = vec![10u8];
    push_string(&mut out, "");
    out.extend_from_slice(entries);
    out.push(0);
    out
}

// ---------------------------------------------------------------------------
// 各标签类型
// ---------------------------------------------------------------------------

#[test]
fn parses_scalars_in_a_compound() {
    let mut entries = Vec::new();
    push_named(&mut entries, 1, "flag", &[0xfau8]); // -6
    push_named(&mut entries, 2, "small", &((-2i16).to_be_bytes()));
    push_named(&mut entries, 3, "count", &42i32.to_be_bytes());
    push_named(&mut entries, 4, "big", &(-7i64).to_be_bytes());
    push_named(&mut entries, 5, "ratio", &(0.5f32).to_bits().to_be_bytes());
    push_named(
        &mut entries,
        6,
        "precise",
        &(1.25f64).to_bits().to_be_bytes(),
    );
    push_named(&mut entries, 8, "label", &{
        let mut text = Vec::new();
        push_string(&mut text, "hello");
        text
    });

    let (name, value) = read_nbt(&root_with(&entries)).expect("parse");
    assert_eq!(name, "");
    assert_eq!(value.get("flag"), Some(&Value::Byte(-6)));
    assert_eq!(value.get("small"), Some(&Value::Short(-2)));
    assert_eq!(value.get("count"), Some(&Value::Int(42)));
    assert_eq!(value.get("big"), Some(&Value::Long(-7)));
    assert_eq!(value.get("ratio"), Some(&Value::Float(0.5)));
    assert_eq!(value.get("precise"), Some(&Value::Double(1.25)));
    assert_eq!(value.get("label"), Some(&Value::String("hello".to_owned())));
    assert_eq!(value.get("absent"), None);
}

#[test]
fn parses_arrays_lists_and_nested_compounds() {
    let mut entries = Vec::new();
    push_named(&mut entries, 7, "bytes", &{
        let mut payload = 2i32.to_be_bytes().to_vec();
        payload.extend_from_slice(&[1u8, 2]);
        payload
    });
    push_named(&mut entries, 11, "ints", &{
        let mut payload = 2i32.to_be_bytes().to_vec();
        payload.extend_from_slice(&5i32.to_be_bytes());
        payload.extend_from_slice(&(-6i32).to_be_bytes());
        payload
    });
    push_named(&mut entries, 12, "longs", &{
        let mut payload = 1i32.to_be_bytes().to_vec();
        payload.extend_from_slice(&1234567890123i64.to_be_bytes());
        payload
    });
    push_named(&mut entries, 9, "words", &{
        let mut payload = vec![8u8];
        payload.extend_from_slice(&2i32.to_be_bytes());
        push_string(&mut payload, "a");
        push_string(&mut payload, "b");
        payload
    });
    push_named(&mut entries, 10, "child", &{
        let mut payload = vec![3u8];
        push_string(&mut payload, "x");
        payload.extend_from_slice(&9i32.to_be_bytes());
        payload.push(0);
        payload
    });

    let (_, value) = read_nbt(&root_with(&entries)).expect("parse");
    assert_eq!(value.get("bytes"), Some(&Value::ByteArray(vec![1, 2])));
    assert_eq!(value.get("ints"), Some(&Value::IntArray(vec![5, -6])));
    assert_eq!(
        value.get("longs"),
        Some(&Value::LongArray(vec![1_234_567_890_123]))
    );
    assert_eq!(
        value.get("words"),
        Some(&Value::List(vec![
            Value::String("a".to_owned()),
            Value::String("b".to_owned()),
        ]))
    );
    let child = value.get("child").expect("child");
    assert_eq!(child.get("x"), Some(&Value::Int(9)));
}

#[test]
fn empty_list_and_empty_compound_are_accepted() {
    let mut entries = Vec::new();
    push_named(&mut entries, 9, "nothing", &{
        let mut payload = vec![0u8]; // TAG_End 的列表
        payload.extend_from_slice(&0i32.to_be_bytes());
        payload
    });
    push_named(&mut entries, 10, "empty", &[0u8]);
    let (_, value) = read_nbt(&root_with(&entries)).expect("parse");
    assert_eq!(value.get("nothing"), Some(&Value::List(Vec::new())));
    assert_eq!(value.get("empty"), Some(&Value::Compound(Vec::new())));
}

// ---------------------------------------------------------------------------
// 拒绝路径
// ---------------------------------------------------------------------------

#[test]
fn rejects_root_that_is_not_a_named_compound() {
    let mut bytes = vec![8u8];
    push_string(&mut bytes, "root");
    push_string(&mut bytes, "text");
    assert_eq!(read_nbt(&bytes), Err(NbtError::RootNotNamedCompound));
}

#[test]
fn rejects_truncated_input() {
    let bytes = root_with(&{
        let mut entries = Vec::new();
        push_named(&mut entries, 3, "count", &[0, 0]); // 应为 4 字节，只有 2 字节
        entries
    });
    assert_eq!(read_nbt(&bytes), Err(NbtError::UnexpectedEof));
}

#[test]
fn rejects_unknown_tag_type() {
    // 未知的标签 99 带上名字字段后才轮到载荷解析。
    let bytes = root_with(&{
        let mut entries = vec![99u8];
        entries.extend_from_slice(&0u16.to_be_bytes());
        entries
    });
    assert_eq!(read_nbt(&bytes), Err(NbtError::UnknownType(99)));
}

#[test]
fn rejects_negative_list_and_array_lengths() {
    let mut entries = Vec::new();
    push_named(&mut entries, 9, "bad", &{
        let mut payload = vec![3u8];
        payload.extend_from_slice(&(-1i32).to_be_bytes());
        payload
    });
    assert_eq!(
        read_nbt(&root_with(&entries)),
        Err(NbtError::NegativeLength)
    );

    let mut entries = Vec::new();
    push_named(&mut entries, 12, "bad", &(-5i32).to_be_bytes());
    assert_eq!(
        read_nbt(&root_with(&entries)),
        Err(NbtError::NegativeLength)
    );
}

#[test]
fn rejects_nesting_beyond_the_depth_limit() {
    const LEVELS: usize = 70;
    // 从最内层往外构造：每层是一个只有一个子复合的具名复合。
    let mut bytes = vec![10u8];
    push_string(&mut bytes, "");
    for _ in 0..LEVELS {
        bytes.push(10);
        push_string(&mut bytes, "n");
    }
    bytes.extend(std::iter::repeat_n(0u8, LEVELS + 1));
    assert!(matches!(read_nbt(&bytes), Err(NbtError::DepthLimit(64))));
}

// ---------------------------------------------------------------------------
// gzip 入口
// ---------------------------------------------------------------------------

#[test]
fn reads_gzip_wrapped_nbt_and_rejects_non_gzip() {
    let plain = root_with(&{
        let mut entries = Vec::new();
        push_named(&mut entries, 3, "v", &1i32.to_be_bytes());
        entries
    });
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    std::io::Write::write_all(&mut encoder, &plain).expect("gzip");
    let compressed = encoder.finish().expect("finish");

    let (_, value) = read_gzip_nbt(&compressed).expect("gzip nbt");
    assert_eq!(value.get("v"), Some(&Value::Int(1)));

    assert_eq!(read_gzip_nbt(&plain), Err(NbtError::InvalidGzip));
}
