#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    CLike,
    Hash,
    Sql,
    Markup,
    Markdown,
    Json,
    Lua,
    Lisp,
    Haskell,
    Diff,
    Ini,
    Plain,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Language {
    pub name: String,
    pub family: Family,
    pub keywords: &'static [&'static str],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Text,
    Markdown,
    Image,
    Svg,
    Mermaid,
    Html,
}

struct Entry {
    name: &'static str,
    family: Family,
    words: &'static [&'static str],
    exts: &'static [&'static str],
    names: &'static [&'static str],
}

const RUST: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern", "false", "fn",
    "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "self", "Self",
    "static", "struct", "super", "trait", "true", "type", "unsafe", "use", "where", "while", "Some", "None", "Ok",
    "Err",
];
const JS: &[&str] = &[
    "async", "await", "break", "case", "catch", "class", "const", "continue", "default", "delete", "do", "else",
    "export", "extends", "false", "finally", "for", "from", "function", "if", "import", "in", "instanceof", "interface",
    "let", "new", "null", "of", "return", "static", "super", "switch", "this", "throw", "true", "try", "type",
    "typeof", "undefined", "var", "void", "while", "yield", "enum", "implements", "readonly", "as",
];
const GO: &[&str] = &[
    "break", "case", "chan", "const", "continue", "default", "defer", "else", "fallthrough", "for", "func", "go",
    "goto", "if", "import", "interface", "map", "package", "range", "return", "select", "struct", "switch", "type",
    "var", "nil", "true", "false",
];
const PY: &[&str] = &[
    "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del", "elif", "else", "except",
    "False", "finally", "for", "from", "global", "if", "import", "in", "is", "lambda", "None", "nonlocal", "not", "or",
    "pass", "raise", "return", "True", "try", "while", "with", "yield", "self",
];
const C: &[&str] = &[
    "auto", "break", "case", "char", "const", "continue", "default", "do", "double", "else", "enum", "extern",
    "float", "for", "goto", "if", "int", "long", "register", "return", "short", "signed", "sizeof", "static", "struct",
    "switch", "typedef", "union", "unsigned", "void", "volatile", "while", "class", "namespace", "public", "private",
    "protected", "template", "typename", "virtual", "new", "delete", "this", "true", "false", "nullptr", "using",
    "include", "define",
];
const JAVA: &[&str] = &[
    "abstract", "break", "case", "catch", "class", "const", "continue", "default", "do", "else", "enum", "extends",
    "final", "finally", "for", "if", "implements", "import", "instanceof", "interface", "new", "null", "package",
    "private", "protected", "public", "return", "static", "super", "switch", "this", "throw", "throws", "try", "void",
    "while", "true", "false", "var", "val", "fun", "object", "when", "override", "func", "let", "struct", "protocol",
    "extension", "guard", "self", "nil", "string", "int", "bool", "using", "namespace", "async", "await",
];
const SHELL: &[&str] = &[
    "if", "then", "else", "elif", "fi", "for", "while", "until", "do", "done", "case", "esac", "in", "function",
    "return", "export", "local", "readonly", "set", "unset", "echo", "exit", "source", "alias", "true", "false",
    "FROM", "RUN", "CMD", "COPY", "ADD", "ENV", "ARG", "WORKDIR", "EXPOSE", "ENTRYPOINT", "USER", "VOLUME", "LABEL",
];
const RUBY: &[&str] = &[
    "begin", "class", "def", "do", "else", "elsif", "end", "ensure", "false", "for", "if", "in", "module", "next",
    "nil", "not", "or", "and", "redo", "rescue", "retry", "return", "self", "super", "then", "true", "unless",
    "until", "when", "while", "yield", "require", "attr_accessor",
];
const PHP: &[&str] = &[
    "function", "class", "public", "private", "protected", "static", "return", "if", "else", "elseif", "foreach",
    "for", "while", "echo", "new", "null", "true", "false", "namespace", "use", "extends", "implements", "array",
];
const SQL: &[&str] = &[
    "select", "from", "where", "insert", "into", "values", "update", "set", "delete", "create", "table", "drop",
    "alter", "index", "join", "left", "right", "inner", "outer", "on", "as", "and", "or", "not", "null", "is", "in",
    "group", "by", "order", "having", "limit", "offset", "primary", "key", "foreign", "references", "default",
    "unique", "distinct", "union", "case", "when", "then", "else", "end", "begin", "commit", "integer", "text",
    "varchar", "boolean",
];
const LUA: &[&str] = &[
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "if", "in", "local", "nil", "not", "or",
    "repeat", "return", "then", "true", "until", "while",
];
const YAML: &[&str] = &["true", "false", "null", "yes", "no", "on", "off"];
const JSON: &[&str] = &["true", "false", "null"];
const NONE: &[&str] = &[];

const TABLE: &[Entry] = &[
    Entry { name: "Rust", family: Family::CLike, words: RUST, exts: &["rs"], names: &[] },
    Entry { name: "TypeScript", family: Family::CLike, words: JS, exts: &["ts", "mts", "cts"], names: &[] },
    Entry { name: "TypeScript React", family: Family::CLike, words: JS, exts: &["tsx"], names: &[] },
    Entry { name: "JavaScript", family: Family::CLike, words: JS, exts: &["js", "mjs", "cjs"], names: &[] },
    Entry { name: "JavaScript React", family: Family::CLike, words: JS, exts: &["jsx"], names: &[] },
    Entry { name: "Go", family: Family::CLike, words: GO, exts: &["go"], names: &[] },
    Entry { name: "Python", family: Family::Hash, words: PY, exts: &["py", "pyi", "pyw"], names: &[] },
    Entry { name: "C", family: Family::CLike, words: C, exts: &["c", "h"], names: &[] },
    Entry { name: "C++", family: Family::CLike, words: C, exts: &["cpp", "cc", "cxx", "hpp", "hh", "hxx"], names: &[] },
    Entry { name: "Objective-C", family: Family::CLike, words: C, exts: &["m", "mm"], names: &[] },
    Entry { name: "C#", family: Family::CLike, words: JAVA, exts: &["cs"], names: &[] },
    Entry { name: "Java", family: Family::CLike, words: JAVA, exts: &["java"], names: &[] },
    Entry { name: "Kotlin", family: Family::CLike, words: JAVA, exts: &["kt", "kts"], names: &[] },
    Entry { name: "Scala", family: Family::CLike, words: JAVA, exts: &["scala", "sc"], names: &[] },
    Entry { name: "Swift", family: Family::CLike, words: JAVA, exts: &["swift"], names: &[] },
    Entry { name: "Dart", family: Family::CLike, words: JAVA, exts: &["dart"], names: &[] },
    Entry { name: "Groovy", family: Family::CLike, words: JAVA, exts: &["groovy", "gradle"], names: &[] },
    Entry { name: "Proto", family: Family::CLike, words: JAVA, exts: &["proto"], names: &[] },
    Entry { name: "PHP", family: Family::CLike, words: PHP, exts: &["php"], names: &[] },
    Entry { name: "CSS", family: Family::CLike, words: NONE, exts: &["css"], names: &[] },
    Entry { name: "SCSS", family: Family::CLike, words: NONE, exts: &["scss", "sass", "less"], names: &[] },
    Entry { name: "JSON", family: Family::Json, words: JSON, exts: &["json", "jsonc", "json5", "jsonl"], names: &[".eslintrc", ".babelrc", ".prettierrc"] },
    Entry { name: "Markdown", family: Family::Markdown, words: NONE, exts: &["md", "markdown", "mdown", "mkd", "mdx"], names: &[] },
    Entry { name: "HTML", family: Family::Markup, words: NONE, exts: &["html", "htm", "xhtml"], names: &[] },
    Entry { name: "Vue", family: Family::Markup, words: NONE, exts: &["vue"], names: &[] },
    Entry { name: "Svelte", family: Family::Markup, words: NONE, exts: &["svelte"], names: &[] },
    Entry { name: "Astro", family: Family::Markup, words: NONE, exts: &["astro"], names: &[] },
    Entry { name: "XML", family: Family::Markup, words: NONE, exts: &["xml", "svg", "plist", "xsd", "xsl", "xslt", "csproj", "iml"], names: &[] },
    Entry { name: "Mermaid", family: Family::Plain, words: NONE, exts: &["mmd", "mermaid"], names: &[] },
    Entry { name: "SQL", family: Family::Sql, words: SQL, exts: &["sql", "psql", "pgsql", "mysql"], names: &[] },
    Entry { name: "Shell", family: Family::Hash, words: SHELL, exts: &["sh", "bash", "zsh", "fish", "command"], names: &[".zshrc", ".bashrc", ".bash_profile", ".profile", ".zprofile", ".zshenv"] },
    Entry { name: "Dockerfile", family: Family::Hash, words: SHELL, exts: &["dockerfile"], names: &["dockerfile", "containerfile"] },
    Entry { name: "Makefile", family: Family::Hash, words: SHELL, exts: &["mk", "mak"], names: &["makefile", "gnumakefile", "justfile"] },
    Entry { name: "Ruby", family: Family::Hash, words: RUBY, exts: &["rb", "rake", "gemspec", "ru"], names: &["gemfile", "rakefile", "podfile", "brewfile", "fastfile"] },
    Entry { name: "Perl", family: Family::Hash, words: NONE, exts: &["pl", "pm"], names: &[] },
    Entry { name: "PowerShell", family: Family::Hash, words: NONE, exts: &["ps1", "psm1", "psd1"], names: &[] },
    Entry { name: "TOML", family: Family::Hash, words: JSON, exts: &["toml"], names: &["cargo.lock"] },
    Entry { name: "YAML", family: Family::Hash, words: YAML, exts: &["yaml", "yml"], names: &[] },
    Entry { name: "Dotenv", family: Family::Hash, words: NONE, exts: &["env"], names: &[".env"] },
    Entry { name: "Config", family: Family::Ini, words: NONE, exts: &["ini", "cfg", "conf", "properties"], names: &[".editorconfig", ".gitconfig", ".npmrc", "nginx.conf"] },
    Entry { name: "CMake", family: Family::Hash, words: NONE, exts: &["cmake"], names: &["cmakelists.txt"] },
    Entry { name: "Lua", family: Family::Lua, words: LUA, exts: &["lua"], names: &[] },
    Entry { name: "Clojure", family: Family::Lisp, words: NONE, exts: &["clj", "cljs", "cljc", "edn"], names: &[] },
    Entry { name: "Haskell", family: Family::Haskell, words: NONE, exts: &["hs"], names: &[] },
    Entry { name: "Diff", family: Family::Diff, words: NONE, exts: &["diff", "patch"], names: &[] },
    Entry { name: "LaTeX", family: Family::Plain, words: NONE, exts: &["tex", "latex", "sty", "cls"], names: &[] },
    Entry { name: "Plain text", family: Family::Plain, words: NONE, exts: &["txt", "text", "log"], names: &[".gitignore", ".dockerignore", "license", "readme"] },
];

pub const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp", "ico", "svg", "tif", "tiff"];

pub fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or("")
}

pub fn folder_of(path: &str) -> &str {
    match path.rfind('/') {
        Some(0) => "/",
        Some(i) => &path[..i],
        None => "",
    }
}

pub fn ext_of(path: &str) -> String {
    let name = file_name(path).to_ascii_lowercase();
    match name.rfind('.') {
        Some(i) if i > 0 && i + 1 < name.len() => name[i + 1..].to_string(),
        _ => String::new(),
    }
}

fn stem_before_first_dot(name: &str) -> &str {
    let from = usize::from(name.starts_with('.'));
    match name[from..].find('.') {
        Some(i) => &name[..from + i],
        None => "",
    }
}

fn entry_for(path: &str) -> Option<&'static Entry> {
    let name = file_name(path).to_ascii_lowercase();
    let ext = ext_of(path);
    TABLE
        .iter()
        .find(|e| e.names.contains(&name.as_str()))
        .or_else(|| (!ext.is_empty()).then(|| TABLE.iter().find(|e| e.exts.contains(&ext.as_str()))).flatten())
        .or_else(|| {
            let stem = stem_before_first_dot(&name);
            (!stem.is_empty()).then(|| TABLE.iter().find(|e| e.names.contains(&stem))).flatten()
        })
}

pub fn language(path: &str) -> Language {
    if let Some(e) = entry_for(path) {
        return Language { name: e.name.to_string(), family: e.family, keywords: e.words };
    }
    let ext = ext_of(path);
    let name = if ext.is_empty() { "Plain text".to_string() } else { format!(".{ext} file") };
    Language { name, family: Family::Plain, keywords: NONE }
}

pub fn shape(path: &str) -> Shape {
    let ext = ext_of(path);
    match ext.as_str() {
        "svg" => Shape::Svg,
        "mmd" | "mermaid" => Shape::Mermaid,
        "html" | "htm" | "xhtml" => Shape::Html,
        "md" | "markdown" | "mdown" | "mkd" | "mdx" => Shape::Markdown,
        e if IMAGE_EXTS.contains(&e) => Shape::Image,
        _ => Shape::Text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_language_by_name_ext_and_prefix() {
        assert_eq!(language("/a/main.rs").name, "Rust");
        assert_eq!(language("/a/Dockerfile").name, "Dockerfile");
        assert_eq!(language("/a/Dockerfile.dev").name, "Dockerfile");
        assert_eq!(language("/a/Makefile.am").name, "Makefile");
        assert_eq!(language("/a/.zshrc").name, "Shell");
        assert_eq!(language("/a/Cargo.lock").name, "TOML");
        assert_eq!(language("/a/x.weird").name, ".weird file");
        assert_eq!(language("/a/NOEXT").name, "Plain text");
        assert_eq!(language("/a/.env").name, "Dotenv");
    }

    #[test]
    fn ext_ignores_leading_and_trailing_dots() {
        assert_eq!(ext_of("/a/.gitignore"), "");
        assert_eq!(ext_of("/a/file."), "");
        assert_eq!(ext_of("/a/B.PNG"), "png");
    }

    #[test]
    fn shapes() {
        assert_eq!(shape("/a/x.md"), Shape::Markdown);
        assert_eq!(shape("/a/x.png"), Shape::Image);
        assert_eq!(shape("/a/x.svg"), Shape::Svg);
        assert_eq!(shape("/a/x.mmd"), Shape::Mermaid);
        assert_eq!(shape("/a/x.html"), Shape::Html);
        assert_eq!(shape("/a/x.rs"), Shape::Text);
    }

    #[test]
    fn folders() {
        assert_eq!(folder_of("/a/b/c.txt"), "/a/b");
        assert_eq!(folder_of("/c.txt"), "/");
        assert_eq!(file_name("/a/b/c.txt"), "c.txt");
    }
}
