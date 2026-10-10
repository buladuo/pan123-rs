//! 123 云盘 Web 端动态签名（社区俗称 `dykey`）。
//!
//! 该机制由 web 前端在 axios 请求拦截器中注入：对**几乎所有** `/b/api/` 请求，
//! 追加一个「键 = CRC32 十进制、值 = `时间戳-随机数-签名`」的查询参数。
//! 服务端校验失败时会返回 `code = -3` 且 `message` 含 `dykey illegality 1001`，
//! 前端收到该错误后会重新拉取服务器时间并重试。
//!
//! 本模块是对前端 `38823` 模块的等价实现，算法已通过真实抓包样本逐条校验
//! （14/14 全部命中）：
//!
//! ```text
//! 时间片   = 北京时间 "YYYYMMDDHHmm"（12 位数字）
//! 映射串   = 时间片每一位数字 d 映射为 LETTERS[d]
//! key      = crc32(映射串)              // 十进制字符串
//! value    = "{ts}-{nonce}-{crc32(\"{ts}|{nonce}|{path}|web|3|{key}\")}"
//! ```
//!
//! 其中 `path` 形如 `/b/api/file/list/new`（含 `/b/api/` 前缀、**不含**查询串）。

use chrono::{Datelike, FixedOffset, TimeZone, Timelike, Utc};

/// 数字 0-9 → 字母的置乱字母表（顺序与前端完全一致，含重复字母，非笔误）。
const LETTERS: &[u8; 26] = b"adefghlmyijnopkqrstubcvwsz";

/// 北京时区（UTC+8）。签名中的时间片固定使用北京时间，与实际运行机器的时区无关。
const BEIJING_OFFSET_SECS: i32 = 8 * 3600;

/// 服务器时间与本地时间偏差超过该阈值（分钟）时，改用服务器时间。
pub const SERVER_TIME_DRIFT_MINUTES: i64 = 20;

/// 标准 CRC32（多项式 0xEDB88320），输出十进制无符号整数。
///
/// 与 web 端 `i()` 函数、以及 `zlib.crc32` 完全一致。
pub fn crc32(input: &str) -> u32 {
    const POLY: u32 = 0xEDB8_8320;
    let mut crc: u32 = 0xFFFF_FFFF;
    for byte in input.as_bytes() {
        crc ^= *byte as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ POLY
            } else {
                crc >> 1
            };
        }
    }
    crc ^ 0xFFFF_FFFF
}

/// 由 unix 时间戳（秒）生成签名用的「时间片 → 字母映射串」。
fn mapped_time_slice(ts: i64) -> String {
    let offset = FixedOffset::east_opt(BEIJING_OFFSET_SECS).expect("valid fixed offset");
    let beijing = Utc
        .timestamp_opt(ts, 0)
        .single()
        .map(|dt| dt.with_timezone(&offset))
        .unwrap_or_else(|| Utc.timestamp_opt(0, 0).unwrap().with_timezone(&offset));

    let digits = format!(
        "{:04}{:02}{:02}{:02}{:02}",
        beijing.year(),
        beijing.month(),
        beijing.day(),
        beijing.hour(),
        beijing.minute()
    );

    digits
        .bytes()
        .map(|b| {
            let idx = (b - b'0') as usize;
            LETTERS[idx] as char
        })
        .collect()
}

/// 计算一次请求所需的签名参数 `(key, value)`。
///
/// * `path`   —— 请求路径，必须以 `/` 开头（例如 `/b/api/file/list/new`）。
/// * `ts`     —— 参与签名的时间戳（秒）。通常取服务器时间，退化时用本地时间。
/// * `nonce`  —— `0..=10_000_000` 的随机数。
pub fn build(path: &str, ts: i64, nonce: u64) -> (String, String) {
    let key = crc32(&mapped_time_slice(ts)).to_string();
    let payload = format!("{ts}|{nonce}|{path}|web|3|{key}");
    let value = format!("{ts}-{nonce}-{}", crc32(&payload));
    (key, value)
}

/// 复刻前端的「本地时间 / 服务器时间」择一逻辑。
///
/// * `local_ts`      —— 本地计算出的时间戳（见 [`local_timestamp`]）。
/// * `server_ts`     —— 已缓存的服务器时间戳（可选）。
///
/// 当已缓存服务器时间、且与本地时间相差 ≥ [`SERVER_TIME_DRIFT_MINUTES`] 分钟时，
/// 认为本机时钟不可信，改用服务器时间；否则使用本地时间。
pub fn pick_timestamp(local_ts: i64, server_ts: Option<i64>) -> i64 {
    match server_ts {
        Some(server) if (local_ts - server).abs() / 60 >= SERVER_TIME_DRIFT_MINUTES => server,
        _ => local_ts,
    }
}

/// 复刻前端的本地时间戳计算：
/// `round((Date.now() + 60 * 时区偏移分钟 * 1000 + 288e5) / 1000)`
///
/// 对于 UTC+8 机器，结果即 unix 秒；对其它时区则等价于「按 UTC+8 归一化后」的秒值，
/// 与服务器 `/b/api/get/server/time` 返回的 `timestamp` 语义一致。
pub fn local_timestamp() -> i64 {
    let now_ms = Utc::now().timestamp_millis();
    let offset_minutes = chrono::Local::now().offset().local_minus_utc() / 60;
    // JS `getTimezoneOffset()` 与 Rust 的符号相反：东八区 JS 返回 -480。
    let js_tz_offset_minutes = -offset_minutes;
    let value = now_ms + 60 * js_tz_offset_minutes as i64 * 1000 + 28_800_000;
    (value as f64 / 1000.0).round() as i64
}

/// 判断某个路径是否需要注入签名。
///
/// 仅 `/b/api/` 前缀的业务接口需要；`/sapi/`、ucenter 的 `/api/` 以及
/// 少量备份类接口（见 [`is_exempt`]）不需要。
pub fn needs_signature(path: &str) -> bool {
    path.starts_with("/b/api/")
}

/// 前端显式豁免签名的接口（备份相关）。
pub fn is_exempt(path: &str) -> bool {
    const EXEMPT: [&str; 7] = [
        "backup/addPath",
        "backup/deletePath",
        "backup/pause",
        "backup/verify",
        "backup/queryPath",
        "backup/start",
        "backup/logout",
    ];
    EXEMPT.iter().any(|e| path.contains(e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_matches_zlib_reference_vectors() {
        assert_eq!(crc32("123456789"), 0xCBF4_3926);
        assert_eq!(crc32(""), 0);
        assert_eq!(crc32("abc"), 891_568_578);
    }

    #[test]
    fn mapped_slice_matches_web_algorithm() {
        // 2026-10-10 21:14 (北京时间) -> "202610102114" -> "eaeldadaeddg"
        // 对应抓包中的 param key 147975487。
        let ts = 1_791_638_057; // unix 秒，北京时间为 2026-10-10 21:14
        assert_eq!(mapped_time_slice(ts), "eaeldadaeddg");
        assert_eq!(crc32(&mapped_time_slice(ts)), 147_975_487);
    }

    #[test]
    fn signature_matches_captured_samples() {
        // 以下两组取自真实浏览器抓包（key=147975487），逐字段复算后完全一致。
        let ts = 1_791_638_057_i64;
        let key = "147975487";

        let (k1, v1) = (
            key.to_string(),
            format!("{ts}-299846-{}", crc32(&format!("{ts}|299846|/b/api/app/config/get|web|3|{key}"))),
        );
        assert_eq!(v1, "1791638057-299846-1953120580");
        assert_eq!(k1, "147975487");

        let v2 = format!("{ts}-7622468-{}", crc32(&format!("{ts}|7622468|/b/api/file/info|web|3|{key}")));
        assert_eq!(v2, "1791638057-7622468-182366213");
    }

    #[test]
    fn build_is_deterministic_and_well_formed() {
        let (key, value) = build("/b/api/file/list/new", 1_791_638_057, 1_234_567);
        assert_eq!(key, "147975487");
        let parts: Vec<&str> = value.split('-').collect();
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[0], "1791638057");
        assert_eq!(parts[1], "1234567");
        assert!(parts[2].parse::<u32>().is_ok());
    }

    #[test]
    fn signature_scope_rules() {
        assert!(needs_signature("/b/api/file/list/new"));
        assert!(!needs_signature("/sapi/version/phone/show"));
        assert!(!needs_signature("/api/user/qr-code/generate"));
        assert!(is_exempt("/b/api/backup/start"));
    }
}
