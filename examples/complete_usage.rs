use pan123_sdk::*;
use std::path::PathBuf;
use std::sync::Arc;

fn main() -> Result<()> {
    println!("🚀 Pan123 SDK 完整使用示例\n");

    // ============================================
    // 1. 创建客户端（带速率限制）
    // ============================================
    println!("📡 初始化客户端...");

    let rate_limiter_config = RateLimiterConfig {
        api_requests_per_second: 8.0,
        upload_bytes_per_second: None,
        download_bytes_per_second: None,
    };

    let mut client = Pan123Client::with_rate_limiter(None, Some(rate_limiter_config))?;
    println!("  ✓ 客户端初始化成功（速率限制: 8 req/s）\n");

    // ============================================
    // 2. 登录（如果需要）
    // ============================================
    match client.check_token_status() {
        TokenCheckStatus::Valid => {
            println!("✅ Token 有效，已登录\n");
        }
        TokenCheckStatus::Missing | TokenCheckStatus::Invalid => {
            println!("🔐 需要登录，请扫描二维码...");
            client.login_by_qrcode()?;
            println!("  ✓ 登录成功\n");
        }
        TokenCheckStatus::Unreachable(msg) => {
            eprintln!("❌ 网络不可达: {}", msg);
            return Ok(());
        }
    }

    // ============================================
    // 3. 获取用户信息
    // ============================================
    println!("👤 获取用户信息...");
    let user_info = client.get_user_info()?;
    println!("  ✓ 用户信息获取成功\n");

    // ============================================
    // 4. 上传文件（带进度回调和重试）
    // ============================================
    println!("📤 上传文件示例...");

    let progress_callback: ProgressCallback = Arc::new(|event| match event {
        TransferEvent::Started { id, path, total_bytes, .. } => {
            println!("  开始上传: {} ({} bytes)", path.display(), total_bytes.unwrap_or(0));
        }
        TransferEvent::Progress { bytes, total_bytes, .. } => {
            if let Some(total) = total_bytes {
                let percent = (bytes as f64 / total as f64 * 100.0) as u32;
                if bytes % (1024 * 1024) == 0 {
                    println!("  进度: {}% ({}/{})", percent, bytes, total);
                }
            }
        }
        TransferEvent::Retrying { attempt, message, .. } => {
            println!("  ⚠️  重试 #{}: {}", attempt, message);
        }
        TransferEvent::Finished { path, total_bytes, .. } => {
            println!("  ✅ 上传完成: {} ({} bytes)", path.display(), total_bytes);
        }
        TransferEvent::Failed { path, message, .. } => {
            eprintln!("  ❌ 上传失败: {} - {}", path.display(), message);
        }
    });

    let upload_options = UploadOptions {
        transfer: TransferOptions {
            parallelism: 4, // 4 线程并发上传分片
            retry: RetryPolicy {
                max_attempts: 5,
                base_delay_ms: 1000,
                max_delay_ms: 60_000,
                exponential_backoff: true,
                jitter: true,
            },
        },
    };

    // 示例：上传文件（如果文件存在）
    let test_file = PathBuf::from("test_upload.txt");
    if test_file.exists() {
        match client.upload_file_with(
            &test_file,
            0, // 根目录
            DuplicateMode::KeepBoth,
            upload_options,
            Some(progress_callback.clone()),
        ) {
            Ok(file_info) => {
                println!("  ✓ 文件上传成功: ID={}\n", file_info.file_id);
            }
            Err(e) => {
                println!("  ⚠️  跳过上传: {}\n", e);
            }
        }
    } else {
        println!("  ⚠️  测试文件不存在，跳过上传\n");
    }

    // ============================================
    // 5. 下载文件（带断点续传）
    // ============================================
    println!("📥 下载文件示例...");

    let download_options = DownloadOptions {
        resume: true, // 启用断点续传
        transfer: TransferOptions {
            parallelism: 3,
            retry: RetryPolicy::default(),
        },
    };

    // 示例：下载文件（需要有效的 file_id）
    // let file_ids = vec![123456];
    // match client.download_files_with(
    //     &file_ids,
    //     "./downloads",
    //     download_options,
    //     Some(progress_callback.clone()),
    // ) {
    //     Ok(downloaded) => {
    //         println!("  ✓ 文件下载成功: {}\n", downloaded.file_path.display());
    //     }
    //     Err(e) => {
    //         eprintln!("  ❌ 下载失败: {}\n", e);
    //     }
    // }

    println!("  ⚠️  需要有效的 file_id，跳过下载示例\n");

    // ============================================
    // 6. 列出文件
    // ============================================
    println!("📂 列出根目录文件...");
    match client.get_file_list(0, 1, 20) {
        Ok(files) => {
            println!("  找到 {} 个文件/文件夹:", files.len());
            for (i, file) in files.iter().take(5).enumerate() {
                let file_type = if file.is_dir() { "📁" } else { "📄" };
                println!(
                    "    {}. {} {} (ID: {}, 大小: {} bytes)",
                    i + 1,
                    file_type,
                    file.file_name,
                    file.file_id,
                    file.size
                );
            }
            if files.len() > 5 {
                println!("    ... 还有 {} 个文件", files.len() - 5);
            }
            println!();
        }
        Err(e) => {
            eprintln!("  ❌ 列出文件失败: {}\n", e);
        }
    }

    // ============================================
    // 7. 文件操作示例
    // ============================================
    println!("🔧 文件操作示例...");

    // 创建文件夹
    match client.create_folder("测试文件夹", 0) {
        Ok(folder) => {
            println!("  ✓ 文件夹创建成功: ID={}", folder.file_id);

            // 重命名
            if let Err(e) = client.rename_file(folder.file_id, "测试文件夹_重命名") {
                println!("  ⚠️  重命名失败: {}", e);
            } else {
                println!("  ✓ 文件夹重命名成功");
            }

            // 删除（移入回收站）
            if let Err(e) = client.delete_files(&[folder.file_id]) {
                println!("  ⚠️  删除失败: {}", e);
            } else {
                println!("  ✓ 文件夹已移入回收站");
            }
        }
        Err(e) => {
            println!("  ⚠️  创建文件夹失败: {}", e);
        }
    }
    println!();

    // ============================================
    // 8. 错误处理示例
    // ============================================
    println!("🔍 错误处理示例...");

    // 尝试访问不存在的文件
    match client.get_file_info(&[999999999]) {
        Ok(files) if files.is_empty() => {
            println!("  ✓ 正确处理：文件不存在");
        }
        Ok(_) => {
            println!("  ⚠️  意外：文件存在");
        }
        Err(e) => {
            println!("  ✓ 捕获错误: {}", e);
            if e.is_retryable() {
                println!("    → 这是可重试的错误");
            } else if e.is_auth_error() {
                println!("    → 这是认证错误");
            } else if e.is_client_error() {
                println!("    → 这是客户端错误");
            }
        }
    }
    println!();

    // ============================================
    // 9. 性能统计
    // ============================================
    println!("📊 性能特性总结:");
    println!("  ✓ Token 加密存储（AES-256-GCM 或系统密钥链）");
    println!("  ✓ 智能重试机制（指数退避 + 随机抖动）");
    println!("  ✓ 分片并发上传（4 线程并发）");
    println!("  ✓ 断点续传（多维度校验）");
    println!("  ✓ 全局速率限制（令牌桶算法）");
    println!("  ✓ 强类型错误处理（编译时检查）");
    println!();

    println!("✅ 示例运行完成！");

    Ok(())
}
