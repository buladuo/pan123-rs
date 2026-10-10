// Task 4-9 期间 MCP 模块尚无调用方（Task 10 才把 run_server 接入 CLI）。
// 二进制 crate 中未使用的 pub 项会触发 dead_code，故暂时豁免；
// Task 10 完成接线后必须删除本属性。
#![allow(dead_code)]

pub mod protocol;
pub mod tools;
