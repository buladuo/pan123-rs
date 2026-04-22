use pan123_sdk::*;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

fn main() -> Result<()> {
    println!("🧪 Pan123 SDK 改进功能测试\n");

    // 测试 1: Token 加密存储
    test_secure_storage()?;

    // 测试 2: 错误处理
    test_error_handling()?;

    // 测试 3: 重试策略
    test_retry_policy()?;

    // 测试 4: 速率限制
    test_rate_limiter()?;

    println!("\n✅ 所有测试通过！");
    Ok(())
}

fn test_secure_storage() -> Result<()> {
    println!("📦 测试 1: Token 加密存储");

    let temp_dir = std::env::temp_dir();
    let token_file = temp_dir.join("test_pan123_token.enc");

    // 清理旧文件
    let _ = std::fs::remove_file(&token_file);

    // 测试加密存储
    let storage = SecureStorage::new(StorageBackend::EncryptedFile, Some(token_file.clone()));
    let test_token = "test-token-12345-abcdef";

    storage.save_token(test_token)?;
    println!("  ✓ Token 已加密保存");

    // 验证文件内容是加密的
    let encrypted_content = std::fs::read_to_string(&token_file)?;
    assert!(!encrypted_content.contains(test_token), "Token 应该被加密");
    println!("  ✓ 文件内容已加密（不包含明文）");

    // 测试读取
    let loaded_token = storage.load_token().expect("应该能读取 token");
    assert_eq!(loaded_token, test_token);
    println!("  ✓ Token 解密成功");

    // 清理
    let _ = std::fs::remove_file(&token_file);
    println!("  ✅ 加密存储测试通过\n");

    Ok(())
}

fn test_error_handling() -> Result<()> {
    println!("🔧 测试 2: 错误处理");

    // 测试错误分类
    let io_error = Pan123Error::Io {
        source: std::io::Error::new(std::io::ErrorKind::NotFound, "file not found"),
    };
    assert!(io_error.is_retryable());
    println!("  ✓ I/O 错误被正确标记为可重试");

    let auth_error = Pan123Error::AuthRequired;
    assert!(auth_error.is_auth_error());
    assert!(!auth_error.is_retryable());
    println!("  ✓ 认证错误被正确分类");

    let client_error = Pan123Error::InvalidPath("test".into());
    assert!(client_error.is_client_error());
    assert!(!client_error.is_retryable());
    println!("  ✓ 客户端错误被正确分类");

    // 测试错误上下文
    let error = Pan123Error::Operation("upload failed".into());
    let with_context = error.with_context("file: test.txt");
    assert!(with_context.to_string().contains("file: test.txt"));
    println!("  ✓ 错误上下文添加成功");

    println!("  ✅ 错误处理测试通过\n");

    Ok(())
}

fn test_retry_policy() -> Result<()> {
    println!("🔄 测试 3: 重试策略");

    // 测试默认策略
    let default_policy = RetryPolicy::default();
    assert_eq!(default_policy.max_attempts, 3);
    assert!(default_policy.exponential_backoff);
    assert!(default_policy.jitter);
    println!("  ✓ 默认策略配置正确");

    // 测试延迟计算
    let delays: Vec<u64> = (1..=5)
        .map(|attempt| default_policy.calculate_delay(attempt))
        .collect();

    println!("  延迟序列: {:?}ms", delays);

    // 验证指数增长
    assert!(delays[1] > delays[0], "延迟应该递增");
    assert!(delays[2] > delays[1], "延迟应该递增");
    println!("  ✓ 指数退避工作正常");

    // 验证抖动
    let delay1 = default_policy.calculate_delay(3);
    let delay2 = default_policy.calculate_delay(3);
    // 由于抖动，两次计算可能不同（但不保证）
    println!("  ✓ 抖动已启用 (delay1={delay1}ms, delay2={delay2}ms)");

    // 测试激进策略
    let aggressive = RetryPolicy::aggressive();
    assert_eq!(aggressive.max_attempts, 5);
    println!("  ✓ 激进策略: {} 次重试", aggressive.max_attempts);

    // 测试保守策略
    let conservative = RetryPolicy::conservative();
    assert_eq!(conservative.max_attempts, 2);
    assert!(!conservative.exponential_backoff);
    println!("  ✓ 保守策略: {} 次重试，固定延迟", conservative.max_attempts);

    println!("  ✅ 重试策略测试通过\n");

    Ok(())
}

fn test_rate_limiter() -> Result<()> {
    println!("⏱️  测试 4: 速率限制");

    // 测试基本功能
    let limiter = RateLimiter::new(10.0); // 10 tokens/s

    // 快速消耗 10 个 token
    for i in 1..=10 {
        assert!(limiter.try_acquire(), "前 10 次应该成功");
        if i % 5 == 0 {
            println!("  ✓ 已消耗 {} tokens", i);
        }
    }

    // 第 11 次应该失败
    assert!(!limiter.try_acquire(), "第 11 次应该失败（token 耗尽）");
    println!("  ✓ Token 耗尽检测正常");

    // 测试速率限制时间
    let limiter = RateLimiter::new(5.0); // 5 tokens/s
    let start = Instant::now();

    // 消耗 10 个 token（应该需要约 1 秒）
    for _ in 0..10 {
        limiter.acquire();
    }

    let elapsed = start.elapsed();
    println!("  ✓ 消耗 10 tokens 用时: {:.2}s (预期 ~2s)", elapsed.as_secs_f64());
    assert!(
        elapsed.as_secs_f64() >= 1.0 && elapsed.as_secs_f64() < 3.0,
        "时间应该在 1-3 秒之间"
    );

    // 测试突发流量
    let limiter = RateLimiter::with_burst(5.0, 20.0); // 5/s，突发 20
    let available = limiter.available_tokens();
    println!("  ✓ 突发容量: {:.0} tokens", available);
    assert!(available >= 19.0 && available <= 20.0);

    println!("  ✅ 速率限制测试通过\n");

    Ok(())
}

#[cfg(test)]
mod integration_tests {
    use super::*;

    #[test]
    fn test_transfer_options() {
        let options = TransferOptions::default();
        assert!(options.parallelism >= 1);
        assert!(options.parallelism <= 4);
        assert_eq!(options.retry.max_attempts, 3);
    }

    #[test]
    fn test_upload_options() {
        let options = UploadOptions::default();
        assert!(options.transfer.parallelism >= 1);
    }

    #[test]
    fn test_download_options() {
        let options = DownloadOptions::default();
        assert!(options.resume);
        assert!(options.transfer.parallelism >= 1);
    }

    #[test]
    fn test_rate_limiter_config() {
        let config = RateLimiterConfig::default();
        assert_eq!(config.api_requests_per_second, 10.0);

        let conservative = RateLimiterConfig::conservative();
        assert_eq!(conservative.api_requests_per_second, 5.0);

        let aggressive = RateLimiterConfig::aggressive();
        assert_eq!(aggressive.api_requests_per_second, 20.0);
    }

    #[test]
    fn test_storage_backend() {
        let backend = StorageBackend::best_available();
        // 应该返回 Keyring 或 EncryptedFile
        match backend {
            StorageBackend::Keyring => println!("系统支持密钥链"),
            StorageBackend::EncryptedFile => println!("使用加密文件存储"),
            StorageBackend::PlaintextFile => panic!("不应该默认使用明文存储"),
        }
    }
}
