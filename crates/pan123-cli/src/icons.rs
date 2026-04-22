pub fn get_file_icon(file_name: &str, is_dir: bool) -> &'static str {
    if is_dir {
        return "📁";
    }

    let extension = file_name
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_lowercase();

    match extension.as_str() {
        // Archives
        "zip" | "tar" | "gz" | "bz2" | "xz" | "7z" | "rar" | "tgz" => "📦",

        // Images
        "jpg" | "jpeg" | "png" | "gif" | "bmp" | "svg" | "webp" | "ico" => "🖼️",

        // Videos
        "mp4" | "avi" | "mkv" | "mov" | "wmv" | "flv" | "webm" | "m4v" => "🎬",

        // Audio
        "mp3" | "wav" | "flac" | "aac" | "ogg" | "m4a" | "wma" => "🎵",

        // Documents
        "pdf" => "📕",
        "doc" | "docx" => "📘",
        "xls" | "xlsx" => "📗",
        "ppt" | "pptx" => "📙",
        "txt" | "md" | "markdown" => "📄",

        // Code
        "rs" => "🦀",
        "py" => "🐍",
        "js" | "jsx" | "ts" | "tsx" => "📜",
        "java" => "☕",
        "go" => "🐹",
        "c" | "cpp" | "cc" | "h" | "hpp" => "⚙️",
        "html" | "htm" => "🌐",
        "css" | "scss" | "sass" => "🎨",
        "json" | "yaml" | "yml" | "toml" | "xml" => "⚙️",
        "sh" | "bash" | "zsh" | "fish" => "🐚",

        // Executables
        "exe" | "msi" | "app" | "dmg" => "⚡",
        "dll" | "so" | "dylib" => "🔧",

        // Database
        "db" | "sqlite" | "sql" => "🗄️",

        // Default
        _ => "📄",
    }
}
