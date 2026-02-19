use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::Path;
use std::sync::LazyLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Category {
    JavaScript,
    Rust,
    Python,
    Java,
    DotNet,
    Swift,
    Dart,
    Go,
    Build,
    Cache,
}

impl Category {
    pub fn as_str(self) -> &'static str {
        match self {
            Category::JavaScript => "JavaScript",
            Category::Rust => "Rust",
            Category::Python => "Python",
            Category::Java => "Java",
            Category::DotNet => ".NET",
            Category::Swift => "Swift",
            Category::Dart => "Dart",
            Category::Go => "Go",
            Category::Build => "Build",
            Category::Cache => "Cache",
        }
    }
}

#[derive(Debug, Clone)]
pub struct CruftPattern {
    pub dir_name: &'static str,
    pub category: Category,
    pub description: &'static str,
    /// If set, the parent directory must contain one of these sibling files
    /// for the match to be valid (avoids false positives on ambiguous names).
    pub context_files: &'static [&'static str],
}

const PATTERNS: &[CruftPattern] = &[
    CruftPattern {
        dir_name: "node_modules",
        category: Category::JavaScript,
        description: "npm/yarn packages",
        context_files: &[],
    },
    CruftPattern {
        dir_name: ".next",
        category: Category::JavaScript,
        description: "Next.js build cache",
        context_files: &[],
    },
    CruftPattern {
        dir_name: "target",
        category: Category::Rust,
        description: "Cargo build output",
        context_files: &["Cargo.toml"],
    },
    CruftPattern {
        dir_name: ".gradle",
        category: Category::Java,
        description: "Gradle cache",
        context_files: &[],
    },
    CruftPattern {
        dir_name: "__pycache__",
        category: Category::Python,
        description: "Python bytecode cache",
        context_files: &[],
    },
    CruftPattern {
        dir_name: ".mypy_cache",
        category: Category::Python,
        description: "mypy type-check cache",
        context_files: &[],
    },
    CruftPattern {
        dir_name: ".pytest_cache",
        category: Category::Python,
        description: "pytest cache",
        context_files: &[],
    },
    CruftPattern {
        dir_name: ".tox",
        category: Category::Python,
        description: "tox virtualenvs",
        context_files: &[],
    },
    CruftPattern {
        dir_name: ".eggs",
        category: Category::Python,
        description: "setuptools egg cache",
        context_files: &[],
    },
    CruftPattern {
        dir_name: "venv",
        category: Category::Python,
        description: "Python virtualenv",
        context_files: &[],
    },
    CruftPattern {
        dir_name: ".venv",
        category: Category::Python,
        description: "Python virtualenv",
        context_files: &[],
    },
    CruftPattern {
        dir_name: "build",
        category: Category::Build,
        description: "Build output",
        context_files: &["build.gradle", "build.gradle.kts", "CMakeLists.txt", "setup.py", "meson.build"],
    },
    CruftPattern {
        dir_name: "dist",
        category: Category::Build,
        description: "Distribution output",
        context_files: &["package.json", "setup.py", "setup.cfg", "pyproject.toml"],
    },
    CruftPattern {
        dir_name: "Pods",
        category: Category::Swift,
        description: "CocoaPods packages",
        context_files: &[],
    },
    CruftPattern {
        dir_name: ".cache",
        category: Category::Cache,
        description: "Generic cache directory",
        context_files: &[],
    },
    CruftPattern {
        dir_name: ".dart_tool",
        category: Category::Dart,
        description: "Dart tool cache",
        context_files: &[],
    },
    CruftPattern {
        dir_name: ".pub-cache",
        category: Category::Dart,
        description: "Dart pub cache",
        context_files: &[],
    },
    CruftPattern {
        dir_name: "vendor",
        category: Category::Go,
        description: "Go vendored deps",
        context_files: &["go.mod", "composer.json"],
    },
    CruftPattern {
        dir_name: "bin",
        category: Category::DotNet,
        description: ".NET build output",
        context_files: &[".csproj", ".fsproj", ".vbproj"],
    },
    CruftPattern {
        dir_name: "obj",
        category: Category::DotNet,
        description: ".NET intermediate output",
        context_files: &[".csproj", ".fsproj", ".vbproj"],
    },
];

/// HashMap for O(1) lookup by directory name -> index into PATTERNS.
static PATTERN_MAP: LazyLock<HashMap<&'static str, Vec<usize>>> = LazyLock::new(|| {
    let mut map: HashMap<&'static str, Vec<usize>> = HashMap::new();
    for (i, p) in PATTERNS.iter().enumerate() {
        map.entry(p.dir_name).or_default().push(i);
    }
    map
});

/// Check if a directory name matches a cruft pattern.
/// For context-aware patterns, `parent` is checked for sibling files.
pub fn match_cruft(name: &OsStr, parent: &Path) -> Option<&'static CruftPattern> {
    let name_str = name.to_str()?;
    let indices = PATTERN_MAP.get(name_str)?;

    for &idx in indices {
        let pattern = &PATTERNS[idx];
        if pattern.context_files.is_empty() {
            return Some(pattern);
        }
        // Context-aware: check if parent contains any required sibling file
        if has_sibling_file(parent, pattern.context_files) {
            return Some(pattern);
        }
    }
    None
}

fn has_sibling_file(parent: &Path, context_files: &[&str]) -> bool {
    for name in context_files {
        // For extensions like ".csproj", glob for any file with that extension
        if name.starts_with('.') && !name.contains('/') && name.len() > 1 && !name[1..].contains('.') {
            if let Ok(entries) = std::fs::read_dir(parent) {
                for entry in entries.flatten() {
                    if let Some(ext) = entry.path().extension() {
                        if format!(".{}", ext.to_string_lossy()) == *name {
                            return true;
                        }
                    }
                }
            }
        } else if parent.join(name).exists() {
            return true;
        }
    }
    false
}
