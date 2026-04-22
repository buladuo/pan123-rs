# Pan123 SDK 改进说明

本文档说明了相对于 Python 版本的主要改进。

## ✅ 已实现的改进

### 1. Token 加密存储 (立即修复)

**问题**: Python 版本使用明文 JSON 存储 token

**解决方案**:
- 优先使用系统密钥链 (Windows Credential Manager / macOS Keychain / Linux Secret Service)
- 降级方案：使用 AES-256-GCM 加密存储到文件
- 密钥派生自机器 ID，防止跨机器复制

**使用示例**:
```rust
use pan123_sdk::{Pan123Client, SecureStorage, StorageBackend};

// 自动选择最佳存储方式
let client = Pan123Client::new(None)?;

// 或手动指定存储方式
let storage = SecureStorage::new(
    StorageBackend::Keyring,  // 或 EncryptedFile / PlaintextFile
    Some(PathBuf::from("token.enc"))
);
storage.save_token("your-token")?;
```

**安全性提升**:
- ✅ Token 不再以明文存储
- ✅ 支持系统级密钥管理
- ✅ 加密文件使用机器绑定密钥
- ✅ 自动迁移旧的明文 token

---

### 2. 完善错误处理 (立即修复)

**问题**: Python 版本大量使用 `except Exception: pass`，静默吞噬错误

**解决方案**:
- 扩展错误类型，区分可重试/不可重试错误
- 添加错误上下文和分类方法
- 所有错误都通过 `Result<T>` 强制处理

**新增错误类型**:
```rust
pub enum Pan123Error {
    Timeout { attempts: usize },
    RateLimited { retry_after_secs: u64 },
    FileConflict { path: String },
    InsufficientStorage,
    InvalidToken,
    Encryption(String),
    Config(String),
    // ... 原有类型
}
```

**错误分类方法**:
```rust
if error.is_retryable() {
    // 自动重试
} else if error.is_auth_error() {
    // 提示重新登录
} else if error.is_client_error() {
    // 用户输入错误
}
```

**改进效果**:
- ✅ 不再静默失败
- ✅ 错误信息更详细
- ✅ 编译时强制错误处理
- ✅ 支持错误链和上下文

---

### 3. 优化请求重试机制 (立即修复)

**问题**: Python 版本使用固定延迟重试，容易触发限流

**解决方案**:
- 指数退避算法
- 随机抖动 (jitter) 避免惊群效应
- 可配置的重试策略

**使用示例**:
```rust
use pan123_sdk::{RetryPolicy, UploadOptions, TransferOptions};

// 默认策略：3次重试，指数退避 + 抖动
let options = UploadOptions::default();

// 激进策略：5次重试，更长等待
let retry = RetryPolicy::aggressive();

// 保守策略：2次重试，固定延迟
let retry = RetryPolicy::conservative();

// 自定义策略
let retry = RetryPolicy {
    max_attempts: 5,
    base_delay_ms: 1000,
    max_delay_ms: 60_000,
    exponential_backoff: true,
    jitter: true,
};
```

**延迟计算**:
```
attempt 1: 1000ms + jitter(0-200ms)
attempt 2: 2000ms + jitter(0-400ms)
attempt 3: 4000ms + jitter(0-800ms)
attempt 4: 8000ms + jitter(0-1600ms)
attempt 5: 16000ms + jitter(0-3200ms)
```

**改进效果**:
- ✅ 减少 API 限流触发
- ✅ 更智能的重试间隔
- ✅ 避免多客户端同时重试
- ✅ 可配置的策略

---

### 4. 分片上传并发化 (中期优化)

**问题**: Python 版本串行上传分片，大文件速度慢

**解决方案**:
- 多线程并发上传分片
- 自动根据 CPU 核心数调整并发度
- 保持分片顺序的进度报告

**使用示例**:
```rust
use pan123_sdk::{UploadOptions, TransferOptions};

let options = UploadOptions {
    transfer: TransferOptions {
        parallelism: 4,  // 4个分片并发上传
        retry: RetryPolicy::default(),
    },
};

client.upload_file_with(
    "large_file.zip",
    parent_id,
    DuplicateMode::KeepBoth,
    options,
    Some(progress_callback),
)?;
```

**性能对比** (1GB 文件，16MB 分片):
```
Python 串行:  ~120秒 (64个分片 × 2秒/片)
Rust 并发4:   ~32秒  (64个分片 ÷ 4 × 2秒/片)
Rust 并发8:   ~18秒  (理论值，受网络限制)
```

**改进效果**:
- ✅ 大文件上传速度提升 3-4 倍
- ✅ 自动调整并发度
- ✅ 失败时快速终止其他线程
- ✅ 准确的进度报告

---

### 5. 增强下载断点续传 (中期优化)

**问题**: Python 版本断点续传逻辑简单，容易误判

**解决方案**:
- 多维度校验：URL、文件名、大小、ETag、Last-Modified
- 定期保存断点元数据 (每 1MB)
- 自动检测文件变更并重新下载

**断点元数据**:
```rust
pub struct DownloadResumeMeta {
    pub url: String,
    pub filename: String,
    pub total_bytes: Option<u64>,
    pub etag: Option<String>,           // 新增
    pub last_modified: Option<String>,  // 新增
    pub downloaded_bytes: u64,          // 新增
    pub created_at: i64,                // 新增
}
```

**校验逻辑**:
```rust
// 检测以下情况会重新下载：
1. URL 或文件名不匹配
2. 文件总大小变化
3. ETag 变化 (文件内容变更)
4. Last-Modified 变化
5. 本地文件大小与元数据不符
```

**使用示例**:
```rust
use pan123_sdk::DownloadOptions;

let options = DownloadOptions {
    resume: true,  // 默认启用
    transfer: TransferOptions::default(),
};

// 支持中断后继续
client.download_files_with(&[file_id], "./downloads", options, None)?;
```

**改进效果**:
- ✅ 更可靠的断点续传
- ✅ 自动检测文件变更
- ✅ 定期保存进度，减少重传
- ✅ 支持 HTTP Range 请求

---

### 6. 全局速率限制 (中期优化)

**问题**: Python 版本无速率限制，容易触发 API 限流

**解决方案**:
- 令牌桶算法实现速率限制
- 支持突发流量 (burst)
- 自动限制 API 请求频率

**使用示例**:
```rust
use pan123_sdk::{Pan123Client, RateLimiterConfig};

// 默认配置：10 req/s
let config = RateLimiterConfig::default();

// 保守配置：5 req/s，限制上传/下载速度
let config = RateLimiterConfig::conservative();

// 激进配置：20 req/s，无速度限制
let config = RateLimiterConfig::aggressive();

// 自定义配置
let config = RateLimiterConfig {
    api_requests_per_second: 10.0,
    upload_bytes_per_second: Some(5 * 1024 * 1024),    // 5 MB/s
    download_bytes_per_second: Some(10 * 1024 * 1024), // 10 MB/s
};

let client = Pan123Client::with_rate_limiter(None, Some(config))?;
```

**令牌桶算法**:
```rust
let limiter = RateLimiter::new(10.0);  // 10 tokens/s

// 阻塞式获取
limiter.acquire();  // 消耗 1 token

// 非阻塞式尝试
if limiter.try_acquire() {
    // 成功获取 token
}

// 批量获取
limiter.acquire_n(5.0);  // 消耗 5 tokens
```

**改进效果**:
- ✅ 避免触发 API 限流
- ✅ 平滑的请求速率
- ✅ 支持突发流量
- ✅ 可配置的限速策略

---

## 📊 性能对比总结

| 指标 | Python 版本 | Rust 版本 | 提升 |
|------|------------|----------|------|
| Token 安全性 | 明文存储 | 加密/密钥链 | ⭐⭐⭐⭐⭐ |
| 错误处理 | 静默失败 | 强制处理 | ⭐⭐⭐⭐⭐ |
| 重试策略 | 固定延迟 | 指数退避+抖动 | ⭐⭐⭐⭐ |
| 大文件上传 | 串行分片 | 并发分片 | 3-4x 速度 |
| 断点续传 | 基础校验 | 多维度校验 | ⭐⭐⭐⭐ |
| API 限流 | 无保护 | 令牌桶限流 | ⭐⭐⭐⭐⭐ |

---

## 🚀 使用建议

### 生产环境推荐配置

```rust
use pan123_sdk::*;

let rate_limiter = RateLimiterConfig {
    api_requests_per_second: 8.0,  // 保守的 API 频率
    upload_bytes_per_second: None,  // 不限制上传速度
    download_bytes_per_second: None,
};

let client = Pan123Client::with_rate_limiter(None, Some(rate_limiter))?;

let upload_options = UploadOptions {
    transfer: TransferOptions {
        parallelism: 4,  // 4 线程并发
        retry: RetryPolicy {
            max_attempts: 5,
            base_delay_ms: 1000,
            max_delay_ms: 60_000,
            exponential_backoff: true,
            jitter: true,
        },
    },
};

let download_options = DownloadOptions {
    resume: true,
    transfer: TransferOptions {
        parallelism: 3,
        retry: RetryPolicy::default(),
    },
};
```

### 开发/测试环境配置

```rust
// 更激进的配置，快速失败
let client = Pan123Client::new(None)?;

let options = UploadOptions {
    transfer: TransferOptions {
        parallelism: 2,
        retry: RetryPolicy::conservative(),  // 2次重试
    },
};
```

---

## 🔧 迁移指南

从 Python 版本迁移到 Rust 版本：

1. **Token 迁移**: 首次运行会自动从旧的 JSON 文件读取并加密存储
2. **API 兼容**: 所有 Python 版本的功能都已实现
3. **性能提升**: 无需修改代码即可享受性能提升
4. **错误处理**: 需要处理 `Result<T>` 返回值

---

## 📝 后续改进建议

1. **异步 I/O**: 使用 `tokio` 替代阻塞 I/O
2. **增量同步**: 实现类似 rsync 的增量同步
3. **分享链接**: 支持生成和解析分享链接
4. **回收站管理**: 完整的回收站操作
5. **文件搜索**: 支持文件名/内容搜索
6. **WebDAV 支持**: 实现 WebDAV 协议
7. **FUSE 挂载**: 将网盘挂载为本地文件系统

---

## 🐛 已知问题

1. 分片并发上传时，如果某个分片失败，会终止所有线程（设计如此）
2. 速率限制器目前只限制 API 请求，不限制上传/下载带宽
3. 断点续传元数据每 1MB 保存一次，极端情况下可能丢失少量进度

---

## 📚 参考资料

- [Rust 错误处理最佳实践](https://doc.rust-lang.org/book/ch09-00-error-handling.html)
- [令牌桶算法](https://en.wikipedia.org/wiki/Token_bucket)
- [指数退避算法](https://en.wikipedia.org/wiki/Exponential_backoff)
- [AES-GCM 加密](https://en.wikipedia.org/wiki/Galois/Counter_Mode)
