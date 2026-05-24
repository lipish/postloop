//! Integration tests for memmap_fs compatibility with IntentLoop requirements.

use intent::storage::Storage;
use memmap_fs::MemMapFS;
use std::io::{Read, Write};
use tempfile::tempdir;

/// Test 1: KV 存储 - 用于存储会话元数据
#[test]
fn test_kv_session_metadata() {
    let dir = tempdir().unwrap();
    let fs = MemMapFS::init(dir.path()).unwrap();

    // 模拟 IntentLoop 的 SessionSummary 序列化存储
    let session_json = r#"{"id":"test-001","agent_cmd":"claude","cwd":"/tmp","status":"running"}"#;

    // 写入
    fs.set_kv(
        "sessions/test-001".to_string(),
        session_json.as_bytes().to_vec(),
    )
    .unwrap();

    // 读取
    let value = fs.get_kv("sessions/test-001");
    assert!(value.is_some());
    assert_eq!(String::from_utf8(value.unwrap()).unwrap(), session_json);
}

/// Test 2: 流式大对象 - 追加写入 + 流式读取
#[test]
fn test_stream_append_and_read() {
    let dir = tempdir().unwrap();
    let fs = MemMapFS::init(dir.path()).unwrap();

    let key = "sessions/test-001/stdout";

    // 模拟多次追加写入（会话进行中）
    fs.append_stream(key, b"Hello, ").unwrap();
    fs.append_stream(key, b"World!").unwrap();
    fs.append_stream(key, b" This is a test.").unwrap();

    // 流式读取
    let mut reader = fs.open_read(key).unwrap();
    let mut content = String::new();
    reader.read_to_string(&mut content).unwrap();

    assert_eq!(content, "Hello, World! This is a test.");
}

/// Test 3: 全文检索
#[test]
fn test_full_text_search() {
    let dir = tempdir().unwrap();
    let fs = MemMapFS::init(dir.path()).unwrap();

    // 索引一些文本
    fs.index("session-001/prompt", "How do I fix the authentication bug?")
        .unwrap();
    fs.index(
        "session-002/response",
        "The authentication issue is caused by...",
    )
    .unwrap();
    fs.index("session-003/prompt", "Create a new REST API endpoint")
        .unwrap();

    // 搜索
    let hits = fs.search("authentication", 10).unwrap();
    assert!(!hits.is_empty());
    // 应该能找到包含 "authentication" 的文档
}

/// Test 4: KV 存储 CRUD
#[test]
fn test_kv_crud() {
    let dir = tempdir().unwrap();
    let fs = MemMapFS::init(dir.path()).unwrap();

    // 设置值
    fs.set_kv("config/agent".to_string(), b"claude".to_vec())
        .unwrap();

    // 获取值
    let value = fs.get_kv("config/agent");
    assert!(value.is_some());
    assert_eq!(value.unwrap(), b"claude");

    // 删除值
    fs.delete_kv("config/agent".to_string()).unwrap();
    let value = fs.get_kv("config/agent");
    assert!(value.is_none());
}

/// Test 5: 崩溃恢复 - WAL 重放
#[test]
fn test_wal_recovery() {
    let dir = tempdir().unwrap();

    // 第一次打开，写入数据
    {
        let fs = MemMapFS::init(dir.path()).unwrap();
        fs.set_kv("test-key".to_string(), b"test-value".to_vec())
            .unwrap();
        fs.append_stream("stream-key", b"stream data").unwrap();
        // fs 在这里 drop，模拟进程退出
    }

    // 重新打开，验证数据恢复
    {
        let fs = MemMapFS::init(dir.path()).unwrap();
        let value = fs.get_kv("test-key");
        assert!(value.is_some());
        assert_eq!(value.unwrap(), b"test-value");

        let mut reader = fs.open_read("stream-key").unwrap();
        let mut content = String::new();
        reader.read_to_string(&mut content).unwrap();
        assert_eq!(content, "stream data");
    }
}

/// Test 6: IntentLoop stream writer adapter used by PTY capture.
#[test]
fn test_storage_stream_writer() {
    let dir = tempdir().unwrap();
    let storage = Storage::init(dir.path()).unwrap();

    let mut writer = storage.stream_writer("test-001", "stdout");
    writer.write_all(b"hello ").unwrap();
    writer.write_all(b"stream").unwrap();
    writer.flush().unwrap();

    let bytes = storage.read_stream_to_bytes("test-001", "stdout").unwrap();
    assert_eq!(bytes, b"hello stream");
}
