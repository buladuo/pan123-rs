use std::borrow::Cow;
use std::collections::HashMap;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use clap::{Parser, Subcommand, ValueEnum};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use owo_colors::OwoColorize;
use pan123_sdk::config;
use pan123_sdk::models::CwdStore;
use pan123_sdk::{
    DownloadOptions, DuplicateMode, FileInfo, Pan123Client, Pan123Error, ProgressCallback, Result,
    RetryPolicy, TokenCheckStatus, TransferEvent, TransferKind, TransferOptions,
    UploadDirectoryReport, UploadFailureKind, UploadOptions,
};
use rustyline::completion::{Candidate, Completer};
use rustyline::error::ReadlineError;
use rustyline::highlight::Highlighter;
use rustyline::hint::Hinter;
use rustyline::history::DefaultHistory;
use rustyline::validate::Validator;
use rustyline::{CompletionType, Config as RustyConfig, Context, Editor, Helper};

const COMMANDS: &[&str] = &[
    "login", "info", "pwd", "cd", "ls", "tree", "upload", "download", "mkdir", "rename", "mv",
    "cp", "rm", "status", "stat", "find", "refresh", "clear", "shell", "help", "exit", "quit",
];

#[derive(Debug, Parser)]
#[command(
    name = "pan123",
    about = "基于 pan123-sdk 的 123 盘命令行工具",
    version
)]
struct CliArgs {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    Login,
    Info,
    Pwd,
    Cd {
        target: String,
        #[arg(long)]
        id: bool,
    },
    Ls {
        #[arg(short, long)]
        parent: Option<String>,
        #[arg(short, long, default_value_t = 100)]
        limit: u32,
    },
    Tree {
        #[arg(short, long)]
        parent: Option<String>,
        #[arg(short, long, default_value_t = 3)]
        depth: usize,
    },
    Upload {
        local_path: PathBuf,
        #[arg(short, long)]
        parent: Option<String>,
        #[arg(long, value_enum, default_value_t = DuplicateArg::KeepBoth)]
        duplicate: DuplicateArg,
        #[arg(long)]
        jobs: Option<usize>,
        #[arg(long, default_value_t = 3)]
        retries: usize,
    },
    Download {
        targets: Vec<String>,
        #[arg(short = 'd', long, default_value = ".")]
        dir: PathBuf,
        #[arg(long, default_value_t = 3)]
        retries: usize,
    },
    Mkdir {
        folder_name: String,
        #[arg(short, long)]
        parent: Option<String>,
    },
    Rename {
        target: String,
        new_name: String,
    },
    Mv {
        target_parent: String,
        sources: Vec<String>,
    },
    Cp {
        target_parent: String,
        sources: Vec<String>,
    },
    Rm {
        targets: Vec<String>,
    },
    #[command(visible_alias = "stat")]
    Status {
        target: String,
        #[arg(long)]
        json: bool,
    },
    Find {
        query: String,
        #[arg(short, long)]
        parent: Option<String>,
        #[arg(short, long, default_value_t = 5)]
        depth: usize,
        #[arg(long)]
        exact: bool,
        #[arg(long)]
        dir_only: bool,
        #[arg(long)]
        file_only: bool,
    },
    Refresh {
        #[arg(long)]
        all: bool,
    },
    Clear,
    Shell,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum DuplicateArg {
    KeepBoth,
    Overwrite,
    Cancel,
}

impl From<DuplicateArg> for DuplicateMode {
    fn from(value: DuplicateArg) -> Self {
        match value {
            DuplicateArg::KeepBoth => DuplicateMode::KeepBoth,
            DuplicateArg::Overwrite => DuplicateMode::Overwrite,
            DuplicateArg::Cancel => DuplicateMode::Cancel,
        }
    }
}

pub struct Pan123Cli {
    client: Option<Pan123Client>,
    shell_state: Arc<Mutex<ShellState>>,
}

impl Pan123Cli {
    pub fn run_from_env() -> Result<()> {
        let args = CliArgs::parse();
        let mut cli = Self {
            client: None,
            shell_state: Arc::new(Mutex::new(ShellState::new())),
        };
        match args.command {
            Some(Command::Shell) | None => cli.run_interactive(),
            Some(command) => cli.execute(command),
        }
    }

    fn execute(&mut self, command: Command) -> Result<()> {
        match command {
            Command::Login => match self.client_mut()?.check_token_status() {
                TokenCheckStatus::Valid => {
                    println!("{}", "当前登录状态仍然有效。".green());
                    Ok(())
                }
                TokenCheckStatus::Unreachable(message) => Err(Pan123Error::Operation(format!(
                    "网络异常，暂时无法验证登录状态：{message}"
                ))),
                TokenCheckStatus::Missing | TokenCheckStatus::Invalid => {
                    self.client_mut()?.login_by_qrcode()?;
                    println!("{}", "登录成功。".green());
                    Ok(())
                }
            },
            Command::Info => {
                self.ensure_auth()?;
                println!(
                    "{}",
                    serde_json::to_string_pretty(&self.client()?.get_user_info()?)?
                );
                Ok(())
            }
            Command::Pwd => {
                let cwd = config::load_cwd();
                println!(
                    "{} {} {}{}{}",
                    "路径".bright_cyan().bold(),
                    cwd.path.bright_white(),
                    "(ID:".dimmed(),
                    cwd.file_id.to_string().bright_black(),
                    ")".dimmed()
                );
                Ok(())
            }
            Command::Cd { target, id } => {
                self.ensure_auth()?;
                let cwd = config::load_cwd();
                let next = self.resolve_cd_target(&cwd, &target, id)?;
                config::save_cwd(&next)?;
                println!(
                    "{} {} {}{}{}",
                    "已切换到".green(),
                    next.path.bright_blue().bold(),
                    "(ID:".dimmed(),
                    next.file_id.to_string().bright_black(),
                    ")".dimmed()
                );
                Ok(())
            }
            Command::Ls { parent, limit } => {
                self.ensure_auth()?;
                let cwd = config::load_cwd();
                let parent_id = match parent {
                    Some(target) => self.resolve_directory_ref(&cwd, &target)?.file_id,
                    None => cwd.file_id,
                };
                let items = self.client()?.get_file_list(parent_id, 1, limit)?;
                self.print_ls(&items);
                Ok(())
            }
            Command::Tree { parent, depth } => {
                self.ensure_auth()?;
                let cwd = config::load_cwd();
                let resolved = match parent {
                    Some(target) => self.resolve_directory_ref(&cwd, &target)?,
                    None => cwd,
                };
                println!("{}", resolved.path.bright_blue().bold());
                self.print_tree(resolved.file_id, String::new(), 1, depth)?;
                Ok(())
            }
            Command::Upload {
                local_path,
                parent,
                duplicate,
                jobs,
                retries,
            } => {
                self.ensure_auth()?;
                let cwd = config::load_cwd();
                let parent_id = match parent {
                    Some(target) => self.resolve_directory_ref(&cwd, &target)?.file_id,
                    None => cwd.file_id,
                };
                let options = UploadOptions {
                    transfer: build_transfer_options(jobs, retries),
                };
                let progress = CliProgress::new();
                let callback = progress.callback();

                if local_path.is_dir() {
                    let report = self.client()?.upload_directory_with(
                        &local_path,
                        parent_id,
                        duplicate.into(),
                        options,
                        Some(callback),
                    )?;
                    progress.finish();
                    self.print_upload_report(&report);
                } else {
                    let info = self.client()?.upload_file_with(
                        &local_path,
                        parent_id,
                        duplicate.into(),
                        options,
                        Some(callback),
                    )?;
                    progress.finish();
                    println!(
                        "{} {} {}{}{}",
                        "已上传".green(),
                        info.file_name.bright_white(),
                        "(ID:".dimmed(),
                        info.file_id.to_string().bright_black(),
                        ")".dimmed()
                    );
                }
                Ok(())
            }
            Command::Download {
                targets,
                dir,
                retries,
            } => {
                self.ensure_auth()?;
                let cwd = config::load_cwd();
                let file_ids = self
                    .resolve_item_refs(&cwd, &targets)?
                    .into_iter()
                    .map(|item| item.file_id)
                    .collect::<Vec<_>>();
                let progress = CliProgress::new();
                let file = self.client()?.download_files_with(
                    &file_ids,
                    &dir,
                    DownloadOptions {
                        transfer: build_transfer_options(None, retries),
                        resume: true,
                    },
                    Some(progress.callback()),
                )?;
                progress.finish();
                println!(
                    "{} {} {}{}{}",
                    "已保存到".green(),
                    file.file_path.display().to_string().bright_white(),
                    "(".dimmed(),
                    human_size(file.size).bright_yellow(),
                    ")".dimmed()
                );
                Ok(())
            }
            Command::Mkdir {
                folder_name,
                parent,
            } => {
                self.ensure_auth()?;
                let cwd = config::load_cwd();
                let parent_id = match parent {
                    Some(target) => self.resolve_directory_ref(&cwd, &target)?.file_id,
                    None => cwd.file_id,
                };
                let info = self.client()?.create_folder(&folder_name, parent_id)?;
                println!(
                    "{} {} {}{}{}",
                    "已创建目录".green(),
                    info.file_name.bright_blue(),
                    "(ID:".dimmed(),
                    info.file_id.to_string().bright_black(),
                    ")".dimmed()
                );
                Ok(())
            }
            Command::Rename { target, new_name } => {
                self.ensure_auth()?;
                let cwd = config::load_cwd();
                let item = self.resolve_item_ref(&cwd, &target)?;
                self.client()?.rename_file(item.file_id, &new_name)?;
                println!(
                    "{} {} {} {}",
                    "已重命名".green(),
                    item.file_name.bright_white(),
                    "->".dimmed(),
                    new_name.bright_white()
                );
                Ok(())
            }
            Command::Mv {
                target_parent,
                sources,
            } => {
                self.ensure_auth()?;
                let cwd = config::load_cwd();
                let target_parent = self.resolve_directory_ref(&cwd, &target_parent)?;
                let file_ids = self
                    .resolve_item_refs(&cwd, &sources)?
                    .into_iter()
                    .map(|item| item.file_id)
                    .collect::<Vec<_>>();
                self.client()?
                    .move_files(&file_ids, target_parent.file_id)?;
                println!(
                    "{} {} {}",
                    "已移动".green(),
                    file_ids.len().to_string().bright_white(),
                    format!("个条目到 {}", target_parent.path).bright_blue()
                );
                Ok(())
            }
            Command::Cp {
                target_parent,
                sources,
            } => {
                self.ensure_auth()?;
                let cwd = config::load_cwd();
                let target_parent = self.resolve_directory_ref(&cwd, &target_parent)?;
                let file_ids = self
                    .resolve_item_refs(&cwd, &sources)?
                    .into_iter()
                    .map(|item| item.file_id)
                    .collect::<Vec<_>>();
                self.client()?
                    .copy_files(&file_ids, target_parent.file_id)?;
                println!(
                    "{} {} {}",
                    "已复制".green(),
                    file_ids.len().to_string().bright_white(),
                    format!("个条目到 {}", target_parent.path).bright_blue()
                );
                Ok(())
            }
            Command::Rm { targets } => {
                self.ensure_auth()?;
                let cwd = config::load_cwd();
                let file_ids = self
                    .resolve_item_refs(&cwd, &targets)?
                    .into_iter()
                    .map(|item| item.file_id)
                    .collect::<Vec<_>>();
                self.client()?.delete_files(&file_ids)?;
                println!(
                    "{} {} {}",
                    "已移动".yellow(),
                    file_ids.len().to_string().bright_white(),
                    "个条目到回收站".yellow()
                );
                Ok(())
            }
            Command::Status { target, json } => {
                self.ensure_auth()?;
                let cwd = config::load_cwd();
                let item = self.resolve_item_ref(&cwd, &target)?;
                self.print_status(&item, json)
            }
            Command::Find {
                query,
                parent,
                depth,
                exact,
                dir_only,
                file_only,
            } => {
                self.ensure_auth()?;
                let cwd = config::load_cwd();
                let parent = match parent {
                    Some(target) => self.resolve_directory_ref(&cwd, &target)?,
                    None => cwd,
                };
                let matcher = build_find_matcher(&query, exact)?;
                let mut results = Vec::new();
                self.find_items(
                    parent.file_id,
                    &parent.path,
                    &matcher,
                    depth,
                    dir_only,
                    file_only,
                    &mut results,
                )?;
                self.print_find_results(&results);
                Ok(())
            }
            Command::Refresh { all } => {
                self.refresh_shell_cache();
                if all {
                    let removed = config::clear_resume_meta_dir()?;
                    println!(
                        "{} {} {}",
                        "缓存已清理。".green(),
                        "已删除".bright_white(),
                        format!("{removed} 个续传元数据文件").bright_yellow()
                    );
                } else {
                    println!("{}", "补全缓存已清理。".green());
                }
                Ok(())
            }
            Command::Clear => clear_screen(),
            Command::Shell => unreachable!(),
        }
    }

    fn run_interactive(&mut self) -> Result<()> {
        println!("{}", "123pan 交互式终端".bright_cyan().bold());
        println!(
            "{}",
            "按 Tab 可补全命令和远端路径。输入 help 查看命令，输入 exit 退出。\n".dimmed()
        );

        let mut editor = build_editor()?;
        editor.set_helper(Some(ShellHelper::new(Arc::clone(&self.shell_state))));

        loop {
            let cwd = config::load_cwd();
            if let Some(helper) = editor.helper_mut() {
                helper.set_cwd(cwd.clone());
            }
            let prompt = format!("pan123 {}> ", cwd.path);
            match editor.readline(&prompt) {
                Ok(line) => {
                    let line = line.trim();
                    if line.is_empty() {
                        continue;
                    }
                    let _ = editor.add_history_entry(line);
                    if matches!(line, "exit" | "quit") {
                        break;
                    }
                    if line == "clear" {
                        clear_screen()?;
                        continue;
                    }
                    if matches!(line, "help" | "?") {
                        self.print_help();
                        continue;
                    }

                    let safe_line = if cfg!(windows) {
                        line.replace('\\', "\\\\")
                    } else {
                        line.to_string()
                    };
                    let args = match shlex::split(&safe_line) {
                        Some(v) => v,
                        None => {
                            eprintln!("{}", "命令行解析失败".red());
                            continue;
                        }
                    };
                    let argv = std::iter::once("pan123".to_string())
                        .chain(args.into_iter())
                        .collect::<Vec<_>>();
                    match CliArgs::try_parse_from(argv) {
                        Ok(parsed) => {
                            if let Some(command) = parsed.command
                                && let Err(err) = self.execute(command)
                            {
                                eprintln!("{} {}", "错误:".red().bold(), err);
                            }
                        }
                        Err(err) => eprintln!("{}", err.to_string().red()),
                    }
                }
                Err(ReadlineError::Interrupted) => {
                    println!();
                    continue;
                }
                Err(ReadlineError::Eof) => {
                    println!();
                    break;
                }
                Err(err) => return Err(Pan123Error::Operation(err.to_string())),
            }
        }
        Ok(())
    }

    fn print_help(&self) {
        println!(
            "{}\n  {}\n  {}\n  {}\n  {}\n  {}\n  {}\n  {}\n  {}\n  {}\n  {}\n  {}\n  {}\n  {}\n  {}\n\n{}",
            "命令".bright_cyan().bold(),
            "login".bright_magenta(),
            "info".bright_magenta(),
            "pwd".bright_magenta(),
            "cd <target> [--id]".bright_magenta(),
            "ls [-p REF] [-l LIMIT]".bright_magenta(),
            "tree [-p REF] [-d DEPTH]".bright_magenta(),
            "upload <path> [-p REF] [--jobs N] [--retries N]".bright_magenta(),
            "download <REF>... [-d DIR] [--retries N]".bright_magenta(),
            "mkdir <name> [-p REF]".bright_magenta(),
            "rename <REF> <new_name>".bright_magenta(),
            "mv <target_ref> <source_ref>...".bright_magenta(),
            "cp <target_ref> <source_ref>...".bright_magenta(),
            "rm <REF>...".bright_magenta(),
            "status|stat <REF> [--json], find <QUERY> [-p REF], refresh [--all], clear".bright_magenta(),
            "REF 可以是 file_id、id:123、当前目录下的名字，或者 /docs/a.txt、../tmp 这样的路径。find 支持普通包含、通配符 (*.zip) 和基础正则 re:^code。".dimmed()
        );
    }

    fn ensure_auth(&mut self) -> Result<()> {
        match self.client_mut()?.check_token_status() {
            TokenCheckStatus::Valid => Ok(()),
            TokenCheckStatus::Missing | TokenCheckStatus::Invalid => {
                println!("{}", "需要登录，正在启动二维码登录...".yellow());
                self.client_mut()?.login_by_qrcode()
            }
            TokenCheckStatus::Unreachable(message) => Err(Pan123Error::Operation(format!(
                "网络异常，暂时无法验证登录状态：{message}"
            ))),
        }
    }

    fn client(&self) -> Result<&Pan123Client> {
        self.client
            .as_ref()
            .ok_or_else(|| Pan123Error::Operation("client not initialized".into()))
    }

    fn client_mut(&mut self) -> Result<&mut Pan123Client> {
        if self.client.is_none() {
            self.client = Some(Pan123Client::new(None)?);
        }
        self.client
            .as_mut()
            .ok_or_else(|| Pan123Error::Operation("client not initialized".into()))
    }

    fn refresh_shell_cache(&self) {
        if let Ok(mut state) = self.shell_state.lock() {
            state.clear_remote_cache();
        }
    }

    fn resolve_cd_target(
        &self,
        cwd: &CwdStore,
        target: &str,
        treat_as_id: bool,
    ) -> Result<CwdStore> {
        let target = normalize_remote_ref(target);
        if target == "/" {
            return Ok(CwdStore::default());
        }
        if target == "." {
            return Ok(cwd.clone());
        }
        if target == ".." {
            if cwd.file_id == 0 {
                return Ok(CwdStore::default());
            }
            let current = self
                .client()?
                .get_file_info(&[cwd.file_id])?
                .into_iter()
                .next()
                .ok_or_else(|| Pan123Error::NotFound(format!("目录 {}", cwd.file_id)))?;
            return Ok(CwdStore {
                file_id: current.parent_file_id,
                path: parent_path(&cwd.path),
            });
        }
        if treat_as_id {
            let file_id = target
                .parse::<u64>()
                .map_err(|_| Pan123Error::InvalidPath(target.clone()))?;
            let info = self
                .client()?
                .get_file_info(&[file_id])?
                .into_iter()
                .next()
                .ok_or_else(|| Pan123Error::NotFound(target.clone()))?;
            if !info.is_dir() {
                return Err(Pan123Error::Operation(format!(
                    "{} 不是目录",
                    info.file_name
                )));
            }
            return Ok(CwdStore {
                file_id: info.file_id,
                path: self.path_from_file_id(info.file_id)?,
            });
        }
        self.resolve_directory_ref(cwd, &target)
    }

    fn resolve_directory_ref(&self, cwd: &CwdStore, target: &str) -> Result<CwdStore> {
        let target = normalize_remote_ref(target);
        if target == "/" {
            return Ok(CwdStore::default());
        }
        if target == "." {
            return Ok(cwd.clone());
        }
        if target == ".." {
            if cwd.file_id == 0 {
                return Ok(CwdStore::default());
            }
            let current = self
                .client()?
                .get_file_info(&[cwd.file_id])?
                .into_iter()
                .next()
                .ok_or_else(|| Pan123Error::NotFound(format!("目录 {}", cwd.file_id)))?;
            return Ok(CwdStore {
                file_id: current.parent_file_id,
                path: parent_path(&cwd.path),
            });
        }
        if let Some(id) = parse_id_ref(&target) {
            let info = self
                .client()?
                .get_file_info(&[id])?
                .into_iter()
                .next()
                .ok_or_else(|| Pan123Error::NotFound(target.clone()))?;
            if !info.is_dir() {
                return Err(Pan123Error::Operation(format!(
                    "{} 不是目录",
                    info.file_name
                )));
            }
            return Ok(CwdStore {
                file_id: info.file_id,
                path: self.path_from_file_id(info.file_id)?,
            });
        }
        self.resolve_path_like_ref(cwd, &target)
    }

    fn resolve_item_ref(&self, cwd: &CwdStore, target: &str) -> Result<FileInfo> {
        let target = normalize_remote_ref(target);
        if let Some(id) = parse_id_ref(&target) {
            return self
                .client()?
                .get_file_info(&[id])?
                .into_iter()
                .next()
                .ok_or_else(|| Pan123Error::NotFound(target.clone()));
        }
        if is_path_like(&target) {
            let resolved_dir = self.resolve_directory_base(cwd, &target)?;
            let file_name =
                final_segment(&target).ok_or_else(|| Pan123Error::InvalidPath(target.clone()))?;
            return self
                .find_child_by_name(resolved_dir.file_id, file_name)?
                .ok_or_else(|| Pan123Error::NotFound(target.clone()));
        }
        self.find_child_by_name(cwd.file_id, &target)?
            .ok_or_else(|| Pan123Error::NotFound(target.clone()))
    }

    fn resolve_item_refs(&self, cwd: &CwdStore, targets: &[String]) -> Result<Vec<FileInfo>> {
        targets
            .iter()
            .map(|target| self.resolve_item_ref(cwd, target))
            .collect()
    }

    fn resolve_path_like_ref(&self, cwd: &CwdStore, target: &str) -> Result<CwdStore> {
        let target = normalize_remote_ref(target);
        let mut current = if target.starts_with('/') {
            CwdStore::default()
        } else {
            cwd.clone()
        };
        for segment in split_segments(&target) {
            match segment {
                "." => {}
                ".." => current = self.resolve_cd_target(&current, "..", false)?,
                other => {
                    let child = self
                        .find_child_by_name(current.file_id, other)?
                        .ok_or_else(|| Pan123Error::NotFound(target.clone()))?;
                    if !child.is_dir() {
                        return Err(Pan123Error::Operation(format!(
                            "{} 不是目录",
                            child.file_name
                        )));
                    }
                    current = CwdStore {
                        file_id: child.file_id,
                        path: join_path(&current.path, &child.file_name),
                    };
                }
            }
        }
        Ok(current)
    }

    fn resolve_directory_base(&self, cwd: &CwdStore, target: &str) -> Result<CwdStore> {
        let target = normalize_remote_ref(target);
        let segments = split_segments(&target);
        if segments.is_empty() {
            return Ok(if target.starts_with('/') {
                CwdStore::default()
            } else {
                cwd.clone()
            });
        }
        let parent_segments = &segments[..segments.len().saturating_sub(1)];
        if parent_segments.is_empty() {
            return Ok(if target.starts_with('/') {
                CwdStore::default()
            } else {
                cwd.clone()
            });
        }
        let parent_target = if target.starts_with('/') {
            format!("/{}", parent_segments.join("/"))
        } else {
            parent_segments.join("/")
        };
        self.resolve_path_like_ref(cwd, &parent_target)
    }

    fn find_child_by_name(&self, parent_id: u64, name: &str) -> Result<Option<FileInfo>> {
        let items = self.client()?.get_file_list(parent_id, 1, 500)?;
        Ok(items.into_iter().find(|item| item.file_name == name))
    }

    fn path_from_file_id(&self, file_id: u64) -> Result<String> {
        let chain = self.client()?.get_path_chain(file_id)?;
        if chain.is_empty() {
            return Ok("/".to_string());
        }
        Ok(format!(
            "/{}",
            chain
                .into_iter()
                .map(|item| item.file_name)
                .collect::<Vec<_>>()
                .join("/")
        ))
    }

    fn print_ls(&self, items: &[FileInfo]) {
        if items.is_empty() {
            println!("{}", "当前目录为空。".dimmed());
            return;
        }
        println!(
            "{}  {}  {}  {}",
            pad_ansi(&"类型".bright_cyan().bold().to_string(), 10),
            pad_ansi(&"ID".bright_cyan().bold().to_string(), 14),
            pad_ansi(&"大小".bright_cyan().bold().to_string(), 12),
            "名称".bright_cyan().bold()
        );
        println!("{}", "─".repeat(72).bright_black());
        for item in items {
            let kind = if item.is_dir() {
                format!("{}", "目录".bright_blue().bold())
            } else {
                format!("{}", "文件".bright_green())
            };
            let size = if item.is_dir() {
                format!("{}", "-".dimmed())
            } else {
                format!("{}", human_size(item.size).bright_yellow())
            };
            let name = if item.is_dir() {
                format!("{}", item.file_name.bright_blue().bold())
            } else {
                format!("{}", item.file_name.bright_white())
            };
            println!(
                "{}  {}  {}  {}",
                pad_ansi(&kind, 10),
                pad_ansi(&format!("{}", item.file_id.to_string().bright_black()), 14),
                pad_ansi(&size, 12),
                name
            );
        }
    }

    fn print_tree(
        &self,
        parent_id: u64,
        prefix: String,
        depth: usize,
        max_depth: usize,
    ) -> Result<()> {
        if depth > max_depth {
            return Ok(());
        }
        let items = self.client()?.get_file_list(parent_id, 1, 500)?;
        let total = items.len();
        for (index, item) in items.into_iter().enumerate() {
            let is_last = index + 1 == total;
            let connector = if is_last { "└──" } else { "├──" };
            let label = if item.is_dir() {
                format!(
                    "{} {} {}",
                    connector.bright_black(),
                    "[目录]".bright_blue(),
                    item.file_name.bright_blue().bold()
                )
            } else {
                format!(
                    "{} {} {}",
                    connector.bright_black(),
                    "[文件]".bright_green(),
                    item.file_name.bright_white()
                )
            };
            println!(
                "{}{} {}{}{}",
                prefix,
                label,
                "(ID:".dimmed(),
                item.file_id.to_string().bright_black(),
                ")".dimmed()
            );
            if item.is_dir() && depth < max_depth {
                let next_prefix = format!("{}{}", prefix, if is_last { "    " } else { "│   " });
                self.print_tree(item.file_id, next_prefix, depth + 1, max_depth)?;
            }
        }
        Ok(())
    }

    fn print_upload_report(&self, report: &UploadDirectoryReport) {
        println!(
            "{} {}  {} {}",
            "上传总结".bright_cyan().bold(),
            format!("成功 {} 个", report.uploaded_count()).green(),
            format!("失败 {} 个", report.failed_count()).red(),
            if report.is_complete_success() {
                "(全部完成)".green().to_string()
            } else {
                "(部分完成)".yellow().to_string()
            }
        );
        if report.failed.is_empty() {
            return;
        }
        let summary = report
            .failure_counts()
            .into_iter()
            .map(|(kind, count)| format!("{}={}", color_failure_kind(kind), count))
            .collect::<Vec<_>>()
            .join(", ");
        println!("{} {}", "失败分类:".yellow().bold(), summary);
        println!("{}", "失败文件:".yellow());
        for failure in &report.failed {
            println!(
                "  [{}] {} {}",
                color_failure_kind(failure.kind),
                failure.path.display().to_string().bright_white(),
                failure.message.dimmed()
            );
        }
    }

    fn print_status(&self, item: &FileInfo, json_output: bool) -> Result<()> {
        if json_output {
            println!("{}", serde_json::to_string_pretty(item)?);
            return Ok(());
        }
        println!("{}", "条目详情".bright_cyan().bold());
        println!(
            "{} {}",
            "名称:".bright_black(),
            item.file_name.bright_white()
        );
        println!(
            "{} {}",
            "类型:".bright_black(),
            if item.is_dir() {
                "目录".bright_blue().bold().to_string()
            } else {
                "文件".bright_green().to_string()
            }
        );
        println!(
            "{} {}",
            "文件 ID:".bright_black(),
            item.file_id.to_string().bright_white()
        );
        println!(
            "{} {}",
            "父目录 ID:".bright_black(),
            item.parent_file_id.to_string().bright_white()
        );
        println!(
            "{} {}",
            "大小:".bright_black(),
            if item.is_dir() {
                "-".dimmed().to_string()
            } else {
                human_size(item.size).bright_yellow().to_string()
            }
        );
        if let Some(status) = item.status {
            println!(
                "{} {}",
                "状态:".bright_black(),
                status.to_string().bright_white()
            );
        }
        if let Some(etag) = &item.etag {
            println!("{} {}", "Etag:".bright_black(), etag.bright_white());
        }
        println!(
            "{} {}",
            "路径:".bright_black(),
            self.path_from_file_id(item.file_id)?.bright_blue()
        );
        println!(
            "{} {}",
            "原始大小:".bright_black(),
            item.size.to_string().bright_white()
        );
        if let Some(extra) = item.extra.as_object()
            && !extra.is_empty()
        {
            println!("{}", "其他字段:".bright_black());
            println!("{}", serde_json::to_string_pretty(&item.extra)?);
        }
        Ok(())
    }

    fn print_find_results(&self, results: &[FileInfoWithPath]) {
        if results.is_empty() {
            println!("{}", "没有找到匹配的条目。".dimmed());
            return;
        }
        println!(
            "{} {}",
            "匹配结果:".bright_cyan().bold(),
            results.len().to_string().bright_white()
        );
        for result in results {
            let kind = if result.item.is_dir() {
                "[目录]".bright_blue().to_string()
            } else {
                "[文件]".bright_green().to_string()
            };
            println!(
                "{} {} {} {}{}{}",
                kind,
                result.path.bright_blue(),
                result.item.file_name.bright_white(),
                "(ID:".dimmed(),
                result.item.file_id.to_string().bright_black(),
                ")".dimmed()
            );
        }
    }

    fn find_items(
        &self,
        parent_id: u64,
        parent_path: &str,
        matcher: &FindMatcher,
        depth: usize,
        dir_only: bool,
        file_only: bool,
        out: &mut Vec<FileInfoWithPath>,
    ) -> Result<()> {
        let items = self.client()?.get_file_list(parent_id, 1, 500)?;
        for item in items {
            let is_match_name = matcher.matches(&item.file_name);
            let allowed = (!dir_only || item.is_dir()) && (!file_only || !item.is_dir());
            if is_match_name && allowed {
                out.push(FileInfoWithPath {
                    path: join_path(parent_path, &item.file_name),
                    item: item.clone(),
                });
            }
            if depth > 0 && item.is_dir() {
                let next_path = join_path(parent_path, &item.file_name);
                self.find_items(
                    item.file_id,
                    &next_path,
                    matcher,
                    depth - 1,
                    dir_only,
                    file_only,
                    out,
                )?;
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
struct ShellCandidate {
    display: String,
    replacement: String,
}

impl Candidate for ShellCandidate {
    fn display(&self) -> &str {
        &self.display
    }

    fn replacement(&self) -> &str {
        &self.replacement
    }
}

#[derive(Clone)]
struct CachedRemoteCandidates {
    fetched_at: Instant,
    items: Vec<ShellCandidate>,
}

#[derive(Clone)]
struct FileInfoWithPath {
    path: String,
    item: FileInfo,
}

enum FindMatcher {
    Exact(String),
    Contains(String),
    Wildcard(String),
    Regex(String),
}

impl FindMatcher {
    fn matches(&self, name: &str) -> bool {
        match self {
            FindMatcher::Exact(v) => name == v,
            FindMatcher::Contains(v) => name.to_lowercase().contains(v),
            FindMatcher::Wildcard(p) => wildcard_match(p, name),
            FindMatcher::Regex(p) => basic_regex_match(p, name),
        }
    }
}

struct ShellState {
    client: Option<Pan123Client>,
    cwd: CwdStore,
    remote_cache: HashMap<(u64, bool), CachedRemoteCandidates>,
}

impl ShellState {
    fn new() -> Self {
        Self {
            client: None,
            cwd: config::load_cwd(),
            remote_cache: HashMap::new(),
        }
    }

    fn set_cwd(&mut self, cwd: CwdStore) {
        self.cwd = cwd;
        self.remote_cache.clear();
    }

    fn clear_remote_cache(&mut self) {
        self.remote_cache.clear();
    }

    fn client(&mut self) -> Option<&mut Pan123Client> {
        if self.client.is_none() {
            self.client = Pan123Client::new(None).ok();
        }
        self.client.as_mut()
    }

    fn remote_candidates(&mut self, prefix: &str, directories_only: bool) -> Vec<ShellCandidate> {
        let cwd = self.cwd.clone();
        let Some(client) = self.client() else {
            return Vec::new();
        };
        let (base_id, base_prefix, needle) = match resolve_completion_base(client, &cwd, prefix) {
            Ok(v) => v,
            Err(_) => return Vec::new(),
        };
        let items = match client.get_file_list(base_id, 1, 500) {
            Ok(items) => items,
            Err(_) => {
                if let Some(cached) = self.remote_cache.get(&(base_id, directories_only))
                    && cached.fetched_at.elapsed() <= remote_cache_ttl()
                {
                    return filter_candidates(cached.items.clone(), &needle);
                }
                return Vec::new();
            }
        };
        let mut candidates = items
            .into_iter()
            .filter(|item| {
                (!directories_only || item.is_dir()) && item.file_name.starts_with(&needle)
            })
            .map(|item| {
                let replacement =
                    build_completion_replacement(&base_prefix, &item.file_name, item.is_dir());
                ShellCandidate {
                    display: replacement.clone(),
                    replacement,
                }
            })
            .collect::<Vec<_>>();
        candidates.sort_by(|a, b| a.replacement.cmp(&b.replacement));
        self.remote_cache.insert(
            (base_id, directories_only),
            CachedRemoteCandidates {
                fetched_at: Instant::now(),
                items: candidates.clone(),
            },
        );
        filter_candidates(candidates, &needle)
    }

    fn local_path_candidates(&self, prefix: &str, directories_only: bool) -> Vec<ShellCandidate> {
        let prefix_path = if prefix.is_empty() { "." } else { prefix };
        let path = Path::new(prefix_path);
        let (base_dir, needle, replacement_base) =
            if prefix.ends_with(std::path::MAIN_SEPARATOR) || prefix.ends_with('/') {
                (
                    PathBuf::from(prefix_path),
                    String::new(),
                    prefix.to_string(),
                )
            } else {
                let parent = path.parent().unwrap_or_else(|| Path::new("."));
                let needle = path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or("")
                    .to_string();
                let replacement_base = if parent == Path::new(".") {
                    String::new()
                } else {
                    format!("{}{}", parent.display(), std::path::MAIN_SEPARATOR)
                };
                (parent.to_path_buf(), needle, replacement_base)
            };
        let entries = match fs::read_dir(&base_dir) {
            Ok(entries) => entries,
            Err(_) => return Vec::new(),
        };
        let mut results = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            let is_dir = path.is_dir();
            if directories_only && !is_dir {
                continue;
            }
            if !name.starts_with(&needle) {
                continue;
            }
            let replacement = if is_dir {
                format!("{}{}{}", replacement_base, name, std::path::MAIN_SEPARATOR)
            } else {
                format!("{}{}", replacement_base, name)
            };
            results.push(ShellCandidate {
                display: replacement.clone(),
                replacement,
            });
        }
        results.sort_by(|a, b| a.replacement.cmp(&b.replacement));
        results
    }
}

struct ShellHelper {
    state: Arc<Mutex<ShellState>>,
}

impl ShellHelper {
    fn new(state: Arc<Mutex<ShellState>>) -> Self {
        Self { state }
    }

    fn set_cwd(&mut self, cwd: CwdStore) {
        if let Ok(mut state) = self.state.lock() {
            state.set_cwd(cwd);
        }
    }
}

impl Helper for ShellHelper {}
impl Validator for ShellHelper {}

impl Hinter for ShellHelper {
    type Hint = String;
}

impl Highlighter for ShellHelper {
    fn highlight_prompt<'b, 's: 'b, 'p: 'b>(&'s self, prompt: &'p str, _: bool) -> Cow<'b, str> {
        Cow::Owned(prompt.to_string())
    }

    fn highlight_hint<'h>(&self, hint: &'h str) -> Cow<'h, str> {
        Cow::Owned(hint.dimmed().to_string())
    }

    fn highlight_candidate<'c>(&self, candidate: &'c str, _: CompletionType) -> Cow<'c, str> {
        if COMMANDS.contains(&candidate) {
            return Cow::Owned(candidate.bright_magenta().bold().to_string());
        }
        if candidate.ends_with('/') || candidate.ends_with(std::path::MAIN_SEPARATOR) {
            return Cow::Owned(candidate.bright_blue().bold().to_string());
        }
        Cow::Owned(candidate.bright_white().to_string())
    }
}

impl Completer for ShellHelper {
    type Candidate = ShellCandidate;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        _: &Context<'_>,
    ) -> rustyline::Result<(usize, Vec<Self::Candidate>)> {
        let (start, token) = extract_token(line, pos);
        let tokens = split_line_tokens(&line[..start]);
        let prev = tokens.last().map(String::as_str);
        let candidates = if tokens.is_empty() {
            command_candidates(&token)
        } else {
            let command = tokens[0].as_str();
            let positionals = positional_tokens(&tokens[1..]);
            match command {
                "upload" if is_option_value(prev, &["-p", "--parent"]) => self
                    .state
                    .lock()
                    .ok()
                    .map(|mut s| s.remote_candidates(&token, true))
                    .unwrap_or_default(),
                "upload" if positionals.is_empty() => self
                    .state
                    .lock()
                    .ok()
                    .map(|s| s.local_path_candidates(&token, false))
                    .unwrap_or_default(),
                "download" if is_option_value(prev, &["-d", "--dir"]) => self
                    .state
                    .lock()
                    .ok()
                    .map(|s| s.local_path_candidates(&token, true))
                    .unwrap_or_default(),
                "download" | "rename" | "rm" | "status" | "stat" => self
                    .state
                    .lock()
                    .ok()
                    .map(|mut s| s.remote_candidates(&token, false))
                    .unwrap_or_default(),
                "cd" => self
                    .state
                    .lock()
                    .ok()
                    .map(|mut s| s.remote_candidates(&token, true))
                    .unwrap_or_default(),
                "ls" | "tree" | "mkdir" | "find" if is_option_value(prev, &["-p", "--parent"]) => {
                    self.state
                        .lock()
                        .ok()
                        .map(|mut s| s.remote_candidates(&token, true))
                        .unwrap_or_default()
                }
                "mv" | "cp" if positionals.is_empty() => self
                    .state
                    .lock()
                    .ok()
                    .map(|mut s| s.remote_candidates(&token, true))
                    .unwrap_or_default(),
                "mv" | "cp" => self
                    .state
                    .lock()
                    .ok()
                    .map(|mut s| s.remote_candidates(&token, false))
                    .unwrap_or_default(),
                _ => command_candidates(&token),
            }
        };
        Ok((start, promote_common_prefix(&token, candidates)))
    }
}

fn build_editor() -> Result<Editor<ShellHelper, DefaultHistory>> {
    let config = RustyConfig::builder()
        .completion_type(CompletionType::List)
        .build();
    Editor::with_config(config).map_err(|err| Pan123Error::Operation(err.to_string()))
}

fn command_candidates(prefix: &str) -> Vec<ShellCandidate> {
    COMMANDS
        .iter()
        .filter(|command| command.starts_with(prefix))
        .map(|command| ShellCandidate {
            display: (*command).to_string(),
            replacement: (*command).to_string(),
        })
        .collect()
}

fn resolve_completion_base(
    client: &Pan123Client,
    cwd: &CwdStore,
    prefix: &str,
) -> Result<(u64, String, String)> {
    let prefix = normalize_remote_ref(prefix);
    if let Some(id) = parse_explicit_id_ref(&prefix) {
        let info = client
            .get_file_info(&[id])?
            .into_iter()
            .next()
            .ok_or_else(|| Pan123Error::NotFound(prefix.clone()))?;
        if !info.is_dir() {
            return Err(Pan123Error::Operation(format!(
                "{} 不是目录",
                info.file_name
            )));
        }
        return Ok((info.file_id, format!("id:{}", info.file_id), String::new()));
    }
    let (base_target, needle, absolute) = split_completion_target(&prefix);
    let mut current_id = if absolute { 0 } else { cwd.file_id };
    let mut current_path = if absolute {
        "/".to_string()
    } else {
        cwd.path.clone()
    };
    for segment in split_segments(&base_target) {
        match segment {
            "." => {}
            ".." => {
                if current_id != 0 {
                    let info = client
                        .get_file_info(&[current_id])?
                        .into_iter()
                        .next()
                        .ok_or_else(|| Pan123Error::NotFound(base_target.clone()))?;
                    current_id = info.parent_file_id;
                    current_path = parent_path(&current_path);
                }
            }
            other => {
                let items = client.get_file_list(current_id, 1, 500)?;
                let child = items
                    .into_iter()
                    .find(|item| item.file_name == other && item.is_dir())
                    .ok_or_else(|| Pan123Error::NotFound(base_target.clone()))?;
                current_id = child.file_id;
                current_path = join_path(&current_path, &child.file_name);
            }
        }
    }
    let base_prefix = if prefix.starts_with('/') {
        if base_target.is_empty() {
            "/".to_string()
        } else {
            format!("/{}/", split_segments(&base_target).join("/"))
        }
    } else if base_target.is_empty() {
        String::new()
    } else {
        format!("{}/", split_segments(&base_target).join("/"))
    };
    Ok((current_id, base_prefix, needle))
}

fn split_completion_target(target: &str) -> (String, String, bool) {
    let absolute = target.starts_with('/');
    let segments = split_segments(target);
    if target.ends_with('/') {
        return (segments.join("/"), String::new(), absolute);
    }
    if let Some((last, parent)) = segments.split_last() {
        (parent.join("/"), (*last).to_string(), absolute)
    } else {
        (String::new(), String::new(), absolute)
    }
}

fn build_completion_replacement(base_prefix: &str, name: &str, is_dir: bool) -> String {
    let mut value = format!("{}{}", base_prefix, name);
    if is_dir {
        value.push('/');
    }
    value
}

fn extract_token(line: &str, pos: usize) -> (usize, String) {
    let prefix = &line[..pos];
    let start = prefix
        .rfind(char::is_whitespace)
        .map(|idx| idx + 1)
        .unwrap_or(0);
    (start, prefix[start..].to_string())
}

fn split_line_tokens(line: &str) -> Vec<String> {
    line.split_whitespace()
        .map(|value| value.to_string())
        .collect()
}

fn positional_tokens(tokens: &[String]) -> Vec<&str> {
    let mut result = Vec::new();
    let mut skip_next = false;
    for token in tokens {
        if skip_next {
            skip_next = false;
            continue;
        }
        if matches!(
            token.as_str(),
            "-p" | "--parent" | "-d" | "--dir" | "--jobs" | "--retries" | "--duplicate"
        ) {
            skip_next = true;
            continue;
        }
        if token.starts_with('-') {
            continue;
        }
        result.push(token.as_str());
    }
    result
}

fn is_option_value(prev: Option<&str>, options: &[&str]) -> bool {
    prev.map(|value| options.contains(&value)).unwrap_or(false)
}

fn color_failure_kind(kind: UploadFailureKind) -> String {
    match kind {
        UploadFailureKind::LocalIo => kind.as_str().yellow().to_string(),
        UploadFailureKind::Network => kind.as_str().bright_magenta().to_string(),
        UploadFailureKind::RemoteApi => kind.as_str().red().to_string(),
        UploadFailureKind::Conflict => kind.as_str().bright_yellow().to_string(),
        UploadFailureKind::Validation => kind.as_str().bright_blue().to_string(),
        UploadFailureKind::Auth => kind.as_str().bright_red().bold().to_string(),
        UploadFailureKind::Unknown => kind.as_str().bright_black().to_string(),
    }
}

fn remote_cache_ttl() -> Duration {
    Duration::from_secs(20)
}

struct CliProgress {
    multi: Arc<MultiProgress>,
    bars: Arc<Mutex<HashMap<String, ProgressBar>>>,
}

impl CliProgress {
    fn new() -> Self {
        Self {
            multi: Arc::new(MultiProgress::new()),
            bars: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn callback(&self) -> ProgressCallback {
        let multi = Arc::clone(&self.multi);
        let bars = Arc::clone(&self.bars);
        Arc::new(move |event| {
            let mut bars = bars.lock().expect("progress bar lock poisoned");
            match event {
                TransferEvent::Started {
                    id,
                    kind,
                    path,
                    total_bytes,
                } => {
                    let pb = multi.add(ProgressBar::new(total_bytes.unwrap_or(0)));
                    pb.set_style(progress_style(total_bytes.is_some()));
                    pb.set_prefix(match kind {
                        TransferKind::Upload => "上传",
                        TransferKind::Download => "下载",
                    });
                    pb.set_message(
                        path.file_name()
                            .and_then(|v| v.to_str())
                            .unwrap_or("传输")
                            .to_string(),
                    );
                    pb.enable_steady_tick(std::time::Duration::from_millis(100));
                    bars.insert(id, pb);
                }
                TransferEvent::Progress {
                    id,
                    bytes,
                    total_bytes,
                    ..
                } => {
                    if let Some(pb) = bars.get(&id) {
                        if let Some(total) = total_bytes {
                            pb.set_length(total);
                        }
                        pb.set_position(bytes);
                    }
                }
                TransferEvent::Retrying {
                    id,
                    attempt,
                    message,
                    ..
                } => {
                    if let Some(pb) = bars.get(&id) {
                        pb.println(format!("重试 {attempt}: {message}"));
                    }
                }
                TransferEvent::Finished {
                    id,
                    path,
                    total_bytes,
                    ..
                } => {
                    if let Some(pb) = bars.remove(&id) {
                        pb.finish_with_message(format!(
                            "{} 完成 ({})",
                            path.file_name().and_then(|v| v.to_str()).unwrap_or("传输"),
                            human_size(total_bytes)
                        ));
                    }
                }
                TransferEvent::Failed {
                    id, path, message, ..
                } => {
                    if let Some(pb) = bars.remove(&id) {
                        pb.abandon_with_message(format!(
                            "{} 失败: {}",
                            path.file_name().and_then(|v| v.to_str()).unwrap_or("传输"),
                            message
                        ));
                    }
                }
            }
        })
    }

    fn finish(&self) {
        if let Ok(mut bars) = self.bars.lock() {
            for (_, pb) in bars.drain() {
                pb.finish_and_clear();
            }
        }
    }
}

fn progress_style(has_total: bool) -> ProgressStyle {
    if has_total {
        ProgressStyle::with_template(
            "{prefix:.bold} [{bar:32.cyan/blue}] {bytes}/{total_bytes} {binary_bytes_per_sec} 剩余 {eta} {msg}",
        )
        .unwrap()
    } else {
        ProgressStyle::with_template(
            "{prefix:.bold} {spinner} {bytes} {binary_bytes_per_sec} {msg}",
        )
        .unwrap()
    }
}

fn build_transfer_options(parallelism: Option<usize>, retries: usize) -> TransferOptions {
    let mut options = TransferOptions::default();
    if let Some(parallelism) = parallelism {
        options.parallelism = parallelism.max(1);
    }
    options.retry = RetryPolicy {
        max_attempts: retries.max(1),
        ..RetryPolicy::default()
    };
    options
}

fn human_size(size: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = size as f64;
    let mut unit = 0usize;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{} {}", size, UNITS[unit])
    } else {
        format!("{value:.2} {}", UNITS[unit])
    }
}

fn join_path(base: &str, segment: &str) -> String {
    if base == "/" {
        format!("/{segment}")
    } else {
        format!("{}/{}", base.trim_end_matches('/'), segment)
    }
}

fn parent_path(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() || trimmed == "/" {
        return "/".to_string();
    }
    let mut parts = trimmed
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    parts.pop();
    if parts.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", parts.join("/"))
    }
}

fn parse_id_ref(target: &str) -> Option<u64> {
    if let Some(rest) = target.strip_prefix("id:") {
        return rest.parse::<u64>().ok();
    }
    if target.contains('/') || target == "." || target == ".." {
        return None;
    }
    target.parse::<u64>().ok()
}

fn parse_explicit_id_ref(target: &str) -> Option<u64> {
    target.strip_prefix("id:")?.parse::<u64>().ok()
}

fn normalize_remote_ref(target: &str) -> String {
    target.replace('\\', "/")
}

fn is_path_like(target: &str) -> bool {
    target.starts_with('/')
        || target.starts_with("./")
        || target.starts_with("../")
        || target.contains('/')
}

fn split_segments(target: &str) -> Vec<&str> {
    target
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect()
}

fn final_segment(target: &str) -> Option<&str> {
    split_segments(target).last().copied()
}

fn build_find_matcher(query: &str, exact: bool) -> Result<FindMatcher> {
    if let Some(pattern) = query.strip_prefix("re:") {
        validate_basic_regex(pattern)?;
        return Ok(FindMatcher::Regex(pattern.to_string()));
    }
    if exact {
        return Ok(FindMatcher::Exact(query.to_string()));
    }
    if query.contains('*') || query.contains('?') {
        return Ok(FindMatcher::Wildcard(query.to_string()));
    }
    Ok(FindMatcher::Contains(query.to_lowercase()))
}

fn validate_basic_regex(pattern: &str) -> Result<()> {
    let chars = pattern.chars().collect::<Vec<_>>();
    if chars.first() == Some(&'*') {
        return Err(Pan123Error::Operation(
            "无效正则: '*' 不能出现在开头".into(),
        ));
    }
    for window in chars.windows(2) {
        if window[0] == '*' && window[1] == '*' {
            return Err(Pan123Error::Operation("无效正则: 暂不支持连续 '*'".into()));
        }
    }
    Ok(())
}

fn pad_ansi(input: &str, width: usize) -> String {
    let plain_len = strip_ansi(input).chars().count();
    if plain_len >= width {
        input.to_string()
    } else {
        format!("{}{}", input, " ".repeat(width - plain_len))
    }
}

fn strip_ansi(input: &str) -> String {
    let mut out = String::new();
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            for next in chars.by_ref() {
                if next == 'm' {
                    break;
                }
            }
        } else {
            out.push(ch);
        }
    }
    out
}

fn wildcard_match(pattern: &str, text: &str) -> bool {
    let p = pattern.as_bytes();
    let t = text.as_bytes();
    let mut dp = vec![vec![false; t.len() + 1]; p.len() + 1];
    dp[0][0] = true;
    for i in 1..=p.len() {
        if p[i - 1] == b'*' {
            dp[i][0] = dp[i - 1][0];
        }
    }
    for i in 1..=p.len() {
        for j in 1..=t.len() {
            dp[i][j] = match p[i - 1] {
                b'*' => dp[i - 1][j] || dp[i][j - 1],
                b'?' => dp[i - 1][j - 1],
                ch => dp[i - 1][j - 1] && ch == t[j - 1],
            };
        }
    }
    dp[p.len()][t.len()]
}

fn basic_regex_match(pattern: &str, text: &str) -> bool {
    if let Some(rest) = pattern.strip_prefix('^') {
        return match_here(rest, text);
    }
    let positions = text
        .char_indices()
        .map(|(idx, _)| idx)
        .chain(std::iter::once(text.len()));
    for start in positions {
        if match_here(pattern, &text[start..]) {
            return true;
        }
    }
    false
}

fn match_here(pattern: &str, text: &str) -> bool {
    if pattern.is_empty() {
        return true;
    }
    if pattern == "$" {
        return text.is_empty();
    }
    let mut chars = pattern.chars();
    let first = chars.next().unwrap_or_default();
    let first_len = first.len_utf8();
    let rest = &pattern[first_len..];
    if let Some(next) = rest.chars().next()
        && next == '*'
    {
        let after_star = &rest[next.len_utf8()..];
        return match_star(first, after_star, text);
    }
    if let Some(ch) = text.chars().next()
        && (first == '.' || first == ch)
    {
        return match_here(rest, &text[ch.len_utf8()..]);
    }
    false
}

fn match_star(ch: char, pattern: &str, text: &str) -> bool {
    let mut current = text;
    loop {
        if match_here(pattern, current) {
            return true;
        }
        let Some(next) = current.chars().next() else {
            return false;
        };
        if ch != '.' && ch != next {
            return false;
        }
        current = &current[next.len_utf8()..];
    }
}

fn filter_candidates(candidates: Vec<ShellCandidate>, needle: &str) -> Vec<ShellCandidate> {
    if needle.is_empty() {
        return candidates;
    }
    candidates
        .into_iter()
        .filter(|candidate| {
            candidate
                .replacement
                .trim_end_matches(['/', '\\'])
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or("")
                .starts_with(needle)
        })
        .collect()
}

fn clear_screen() -> Result<()> {
    print!("\x1B[2J\x1B[H");
    io::stdout().flush()?;
    Ok(())
}

fn promote_common_prefix(token: &str, candidates: Vec<ShellCandidate>) -> Vec<ShellCandidate> {
    if token.is_empty() || candidates.len() <= 1 {
        return candidates;
    }
    let mut iter = candidates
        .iter()
        .map(|candidate| candidate.replacement.as_str());
    let Some(first) = iter.next() else {
        return candidates;
    };
    let prefix = iter.fold(first.to_string(), |current, next| {
        common_prefix(&current, next)
    });
    if prefix.len() > token.len() {
        vec![ShellCandidate {
            display: prefix.clone(),
            replacement: prefix,
        }]
    } else {
        candidates
    }
}

fn common_prefix(left: &str, right: &str) -> String {
    left.chars()
        .zip(right.chars())
        .take_while(|(a, b)| a == b)
        .map(|(ch, _)| ch)
        .collect()
}
