//! File-type icons for the Files browser.
//!
//! Glyphs are the original **Seti-UI** SVGs (jesweed/seti-ui, MIT) with baked
//! colors stripped, tinted with the exact `fontColor` from VS Code's built-in
//! `theme-seti`. Matches the default VS Code Explorer look (same shapes + colors).

use gpui::prelude::*;
use gpui::*;

const ICON: f32 = 16.0;
const FOLDER_HEX: &str = "#dcb67a";

#[derive(Clone, Copy)]
struct Icon {
    svg: &'static str,
    color: Hsla,
}

fn hex(c: &str) -> Hsla {
    let h = c.trim_start_matches('#');
    let n = u32::from_str_radix(h, 16).unwrap_or(0xd4d7d6);
    let r = ((n >> 16) & 0xff) as f32 / 255.0;
    let g = ((n >> 8) & 0xff) as f32 / 255.0;
    let b = (n & 0xff) as f32 / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    if (max - min).abs() < 1e-6 {
        return Hsla { h: 0.0, s: 0.0, l, a: 1.0 };
    }
    let d = max - min;
    let s = if l > 0.5 { d / (2.0 - max - min) } else { d / (max + min) };
    let h = if max == r {
        (g - b) / d + (if g < b { 6.0 } else { 0.0 })
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    Hsla { h: h / 6.0, s, l, a: 1.0 }
}

fn s(svg: &'static str, color: &'static str) -> Icon {
    Icon { svg, color: hex(color) }
}

fn basename_lower(name: &str) -> String {
    std::path::Path::new(name)
        .file_name()
        .and_then(|x| x.to_str())
        .unwrap_or(name)
        .to_ascii_lowercase()
}

fn ext_lower(name: &str) -> String {
    let base = basename_lower(name);
    let compounds = [
        "stylelintrc.json", "stylelintrc.yaml", "stylelintrc.yml", "stylelintrc.js",
        "codeclimate.yml", "eslintrc.yaml", "eslintrc.yml", "eslintrc.json",
        "eslintrc.cjs", "eslintrc.js", "gitlab-ci.yml", "babelrc.cjs", "babelrc.js",
        "smarty.tpl", "erb.html", "html.erb", "npm-debug.log", "php.inc",
        "spec.cjs", "test.cjs", "spec.mjs", "test.mjs", "spec.jsx", "test.jsx",
        "spec.tsx", "test.tsx", "spec.mts", "test.mts", "spec.cts", "test.cts",
        "css.map", "cjs.map", "mjs.map", "js.map", "tf.json", "tfvars.json",
        "spec.js", "test.js", "spec.ts", "test.ts", "d.ts", "d.mts", "d.cts",
    ];
    for c in compounds {
        if base.ends_with(c) {
            return c.to_string();
        }
    }
    std::path::Path::new(&base)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

fn icon_for(name: &str, is_dir: bool) -> Icon {
    if is_dir {
        return Icon { svg: "folder", color: hex(FOLDER_HEX) };
    }
    let base = basename_lower(name);

    // --- fileNames (Seti fileNames + languageId-driven) ---
    if base == "dockerfile" || base == "containerfile" || base.starts_with("dockerfile.") {
        return s("docker", "#519aba");
    }
    if base.starts_with("docker-compose.") || base.starts_with("compose.")
        || matches!(base.as_str(), "docker-compose.yml" | "docker-compose.yaml" | "compose.yml" | "compose.yaml")
    {
        return s("docker", "#f55385");
    }
    if matches!(base.as_str(), "makefile" | "gnumakefile" | "cmakelists.txt") {
        return s("makefile", "#e37933");
    }
    if matches!(base.as_str(), ".gitignore" | ".gitattributes" | ".gitmodules" | ".gitkeep" | ".git-blame-ignore-revs") {
        return s("git", "#41535b");
    }
    if matches!(base.as_str(), "package.json" | "package-lock.json") {
        return s("json", "#cbcb41");
    }
    if base == ".npmrc" {
        return s("npm", "#cbcb41");
    }
    if matches!(base.as_str(), "yarn.lock" | ".yarnrc" | ".yarnrc.yml") {
        return s("yarn", "#519aba");
    }
    if matches!(base.as_str(), "tsconfig.json" | "tsconfig.app.json" | "tsconfig.node.json" | "jsconfig.json") {
        return s("tsconfig", "#519aba");
    }
    if base == "readme" || base.starts_with("readme.") {
        return s("info", "#519aba");
    }
    if matches!(base.as_str(), "license" | "licence" | "copying" | "license.md" | "licence.md" | "license.txt") {
        return s("license", "#cbcb41");
    }
    if base.starts_with("vite.config.") {
        return s("vite", "#cbcb41");
    }
    if base.starts_with("eslint.config.") || base.starts_with(".eslintrc") {
        return s("eslint", "#a074c4");
    }
    if base.starts_with("babel.config.") {
        return s("babel", "#cbcb41");
    }
    if base == "pom.xml" {
        return s("maven", "#cc3e44");
    }
    if matches!(base.as_str(), "cargo.toml" | "cargo.lock" | "rust-toolchain" | "rust-toolchain.toml") {
        return s("config", "#6d8086");
    }
    if matches!(base.as_str(), "go.mod" | "go.sum") {
        return s("go2", "#519aba");
    }
    if base == ".editorconfig" {
        return s("editorconfig", "#6d8086");
    }
    if base.starts_with(".env") {
        return s("config", "#6d8086");
    }

    // --- extensions ---
    match ext_lower(name).as_str() {
        "babelrc.cjs" => s("babel", "#cbcb41"),
        "babelrc.js" => s("babel", "#cbcb41"),
        "cjs.map" => s("javascript", "#cbcb41"),
        "codeclimate.yml" => s("code-climate", "#8dc149"),
        "css.map" => s("css", "#519aba"),
        "erb.html" => s("html", "#cc3e44"),
        "eslintrc.cjs" => s("eslint", "#a074c4"),
        "eslintrc.js" => s("eslint", "#a074c4"),
        "eslintrc.json" => s("eslint", "#a074c4"),
        "eslintrc.yaml" => s("eslint", "#a074c4"),
        "eslintrc.yml" => s("eslint", "#a074c4"),
        "gitlab-ci.yml" => s("gitlab", "#e37933"),
        "html.erb" => s("html", "#cc3e44"),
        "js.map" => s("javascript", "#cbcb41"),
        "mjs.map" => s("javascript", "#cbcb41"),
        "npm-debug.log" => s("npm", "#41535b"),
        "php.inc" => s("php", "#a074c4"),
        "smarty.tpl" => s("smarty", "#cbcb41"),
        "spec.cjs" => s("javascript", "#e37933"),
        "spec.cts" => s("typescript", "#e37933"),
        "spec.js" => s("javascript", "#e37933"),
        "spec.jsx" => s("react", "#e37933"),
        "spec.mjs" => s("javascript", "#e37933"),
        "spec.mts" => s("typescript", "#e37933"),
        "spec.ts" => s("typescript", "#e37933"),
        "spec.tsx" => s("react", "#e37933"),
        "stylelintrc.js" => s("stylelint", "#d4d7d6"),
        "stylelintrc.json" => s("stylelint", "#d4d7d6"),
        "stylelintrc.yaml" => s("stylelint", "#d4d7d6"),
        "stylelintrc.yml" => s("stylelint", "#d4d7d6"),
        "test.cjs" => s("javascript", "#e37933"),
        "test.cts" => s("typescript", "#e37933"),
        "test.js" => s("javascript", "#e37933"),
        "test.jsx" => s("react", "#e37933"),
        "test.mjs" => s("javascript", "#e37933"),
        "test.mts" => s("typescript", "#e37933"),
        "test.ts" => s("typescript", "#e37933"),
        "test.tsx" => s("react", "#e37933"),
        "tf.json" => s("terraform", "#a074c4"),
        "tfvars.json" => s("terraform", "#a074c4"),
        "3dm" => s("svg", "#519aba"),
        "3ds" => s("svg", "#519aba"),
        "ad" => s("argdown", "#519aba"),
        "ai" => s("illustrator", "#cbcb41"),
        "apex" => s("salesforce", "#519aba"),
        "argdown" => s("argdown", "#519aba"),
        "article" => s("go", "#519aba"),
        "asax" => s("html", "#cbcb41"),
        "ascx" => s("html", "#8dc149"),
        "aspx" => s("html", "#519aba"),
        "avi" => s("video", "#f55385"),
        "avif" => s("image", "#a074c4"),
        "babelrc" => s("babel", "#cbcb41"),
        "bash" => s("shell", "#8dc149"),
        "bat" => s("windows", "#519aba"),
        "bicep" => s("bicep", "#519aba"),
        "bowerrc" => s("bower", "#e37933"),
        "bsl" => s("bsl", "#cc3e44"),
        "c" => s("c", "#519aba"),
        "cake" => s("cake", "#cc3e44"),
        "cc" => s("cpp", "#519aba"),
        "cer" => s("lock", "#8dc149"),
        "cert" => s("lock", "#8dc149"),
        "cfc" => s("coldfusion", "#519aba"),
        "cfm" => s("coldfusion", "#519aba"),
        "cjs" => s("javascript", "#cbcb41"),
        "cjsx" => s("react", "#519aba"),
        "class" => s("java", "#519aba"),
        "classpath" => s("java", "#cc3e44"),
        "clj" => s("clojure", "#8dc149"),
        "cljc" => s("clojure", "#8dc149"),
        "cls" => s("salesforce", "#519aba"),
        "cmd" => s("windows", "#519aba"),
        "cmx" => s("ocaml", "#e37933"),
        "cmxa" => s("ocaml", "#e37933"),
        "command" => s("shell", "#8dc149"),
        "component" => s("html", "#e37933"),
        "config" => s("config", "#6d8086"),
        "cpp" => s("cpp", "#519aba"),
        "cr" => s("crystal", "#d4d7d6"),
        "crt" => s("lock", "#8dc149"),
        "cs" => s("c-sharp", "#519aba"),
        "cson" => s("json", "#cbcb41"),
        "css" => s("css", "#519aba"),
        "csv" => s("csv", "#8dc149"),
        "ctp" => s("cake_php", "#cc3e44"),
        "cts" => s("typescript", "#519aba"),
        "cxx" => s("cpp", "#519aba"),
        "d" => s("d", "#cc3e44"),
        "dae" => s("svg", "#519aba"),
        "dart" => s("dart", "#519aba"),
        "direnv" => s("config", "#6d8086"),
        "doc" => s("word", "#519aba"),
        "dockerignore" => s("docker", "#4d5a5e"),
        "docx" => s("word", "#519aba"),
        "ecr" => s("crystal_embedded", "#d4d7d6"),
        "edn" => s("clojure", "#519aba"),
        "ejs" => s("ejs", "#cbcb41"),
        "elm" => s("elm", "#519aba"),
        "eot" => s("font", "#cc3e44"),
        "epp" => s("puppet", "#cbcb41"),
        "erb" => s("html", "#cc3e44"),
        "erl" => s("default", "#cc3e44"),
        "es" => s("javascript", "#cbcb41"),
        "es5" => s("javascript", "#cbcb41"),
        "es7" => s("javascript", "#cbcb41"),
        "eslintignore" => s("eslint", "#4d5a5e"),
        "eslintrc" => s("eslint", "#a074c4"),
        "ex" => s("elixir", "#a074c4"),
        "exs" => s("elixir_script", "#a074c4"),
        "firebaserc" => s("firebase", "#e37933"),
        "fish" => s("shell", "#8dc149"),
        "flac" => s("audio", "#a074c4"),
        "fs" => s("f-sharp", "#519aba"),
        "fsi" => s("f-sharp", "#519aba"),
        "fsx" => s("f-sharp", "#519aba"),
        "gd" => s("godot", "#519aba"),
        "gif" => s("image", "#a074c4"),
        "gitattributes" => s("git", "#41535b"),
        "gitconfig" => s("git", "#41535b"),
        "gitkeep" => s("git", "#41535b"),
        "gitmodules" => s("git", "#41535b"),
        "go" => s("go2", "#519aba"),
        "godot" => s("godot", "#cc3e44"),
        "gql" => s("graphql", "#f55385"),
        "gradle" => s("gradle", "#519aba"),
        "graphql" => s("graphql", "#f55385"),
        "graphqls" => s("graphql", "#f55385"),
        "gsp" => s("grails", "#8dc149"),
        "h" => s("c", "#a074c4"),
        "h++" => s("cpp", "#a074c4"),
        "hack" => s("hacklang", "#e37933"),
        "haml" => s("haml", "#cc3e44"),
        "happenings" => s("happenings", "#519aba"),
        "hh" => s("cpp", "#a074c4"),
        "hpp" => s("cpp", "#a074c4"),
        "hrl" => s("default", "#cc3e44"),
        "hs" => s("haskell", "#a074c4"),
        "htaccess" => s("config", "#6d8086"),
        "htm" => s("html", "#e37933"),
        "html" => s("html", "#e37933"),
        "hx" => s("haxe", "#e37933"),
        "hxml" => s("haxe", "#a074c4"),
        "hxp" => s("haxe", "#519aba"),
        "hxs" => s("haxe", "#cbcb41"),
        "hxx" => s("cpp", "#a074c4"),
        "ico" => s("favicon", "#cbcb41"),
        "ini" => s("config", "#6d8086"),
        "ipynb" => s("notebook", "#519aba"),
        "jade" => s("jade", "#cc3e44"),
        "jar" => s("zip", "#cc3e44"),
        "java" => s("java", "#cc3e44"),
        "jinja" => s("jinja", "#cc3e44"),
        "jinja2" => s("jinja", "#cc3e44"),
        "jpeg" => s("image", "#a074c4"),
        "jpg" => s("image", "#a074c4"),
        "js" => s("javascript", "#cbcb41"),
        "jscsrc" => s("javascript", "#519aba"),
        "jshintrc" => s("javascript", "#519aba"),
        "json" => s("json", "#cbcb41"),
        "json5" => s("json", "#cbcb41"),
        "jsonc" => s("json", "#cbcb41"),
        "key" => s("lock", "#8dc149"),
        "ksh" => s("shell", "#8dc149"),
        "kt" => s("kotlin", "#e37933"),
        "kts" => s("kotlin", "#e37933"),
        "less" => s("less", "#519aba"),
        "lhs" => s("haskell", "#a074c4"),
        "liquid" => s("liquid", "#8dc149"),
        "ls" => s("livescript", "#519aba"),
        "lua" => s("lua", "#519aba"),
        "master" => s("html", "#cbcb41"),
        "md" => s("markdown", "#519aba"),
        "mdo" => s("mdo", "#cc3e44"),
        "mdx" => s("markdown", "#519aba"),
        "mjs" => s("javascript", "#cbcb41"),
        "ml" => s("ocaml", "#e37933"),
        "mli" => s("ocaml", "#e37933"),
        "mov" => s("video", "#f55385"),
        "mp3" => s("audio", "#a074c4"),
        "mp4" => s("video", "#f55385"),
        "mpg" => s("video", "#f55385"),
        "mts" => s("typescript", "#519aba"),
        "mustache" => s("mustache", "#e37933"),
        "nim" => s("nim", "#cbcb41"),
        "nims" => s("nim", "#cbcb41"),
        "nj" => s("nunjucks", "#8dc149"),
        "njk" => s("nunjucks", "#8dc149"),
        "njs" => s("nunjucks", "#8dc149"),
        "npmignore" => s("npm", "#cc3e44"),
        "npmrc" => s("npm", "#cc3e44"),
        "nunj" => s("nunjucks", "#8dc149"),
        "nunjs" => s("nunjucks", "#8dc149"),
        "nunjucks" => s("nunjucks", "#8dc149"),
        "obj" => s("svg", "#519aba"),
        "odata" => s("odata", "#e37933"),
        "ogg" => s("audio", "#a074c4"),
        "ogv" => s("video", "#f55385"),
        "otf" => s("font", "#cc3e44"),
        "pddl" => s("pddl", "#a074c4"),
        "pdf" => s("pdf", "#cc3e44"),
        "pem" => s("lock", "#8dc149"),
        "php" => s("php", "#a074c4"),
        "pipeline" => s("pipeline", "#e37933"),
        "plan" => s("plan", "#8dc149"),
        "png" => s("image", "#a074c4"),
        "pp" => s("puppet", "#cbcb41"),
        "prisma" => s("prisma", "#519aba"),
        "pro" => s("prolog", "#e37933"),
        "ps1" => s("powershell", "#519aba"),
        "psd" => s("photoshop", "#519aba"),
        "psd1" => s("powershell", "#519aba"),
        "psm1" => s("powershell", "#519aba"),
        "purs" => s("purescript", "#d4d7d6"),
        "pxm" => s("image", "#a074c4"),
        "py" => s("python", "#519aba"),
        "pyi" => s("python", "#519aba"),
        "pyw" => s("python", "#519aba"),
        "r" => s("R", "#519aba"),
        "rb" => s("ruby", "#cc3e44"),
        "re" => s("reasonml", "#cc3e44"),
        "res" => s("rescript", "#cc3e44"),
        "resi" => s("rescript", "#f55385"),
        "rmd" => s("R", "#519aba"),
        "rs" => s("rust", "#6d8086"),
        "sass" => s("sass", "#f55385"),
        "sbt" => s("sbt", "#519aba"),
        "scala" => s("scala", "#cc3e44"),
        "scss" => s("sass", "#f55385"),
        "sh" => s("shell", "#8dc149"),
        "slang" => s("crystal_embedded", "#d4d7d6"),
        "slide" => s("go", "#519aba"),
        "slim" => s("slim", "#e37933"),
        "slugignore" => s("config", "#6d8086"),
        "sol" => s("ethereum", "#519aba"),
        "soql" => s("db", "#519aba"),
        "springbeans" => s("spring", "#8dc149"),
        "sql" => s("db", "#f55385"),
        "sss" => s("css", "#519aba"),
        "stache" => s("mustache", "#e37933"),
        "static" => s("config", "#6d8086"),
        "stl" => s("svg", "#519aba"),
        "styl" => s("stylus", "#8dc149"),
        "stylelintignore" => s("stylelint", "#4d5a5e"),
        "stylelintrc" => s("stylelint", "#d4d7d6"),
        "sublime-project" => s("sublime", "#e37933"),
        "sublime-workspace" => s("sublime", "#e37933"),
        "svelte" => s("svelte", "#cc3e44"),
        "svg" => s("svg", "#a074c4"),
        "svgx" => s("image", "#a074c4"),
        "swift" => s("swift", "#e37933"),
        "tex" => s("tex", "#cbcb41"),
        "tf" => s("terraform", "#a074c4"),
        "tfvars" => s("terraform", "#a074c4"),
        "tiff" => s("image", "#a074c4"),
        "tmp" => s("clock", "#6d8086"),
        "toml" => s("config", "#6d8086"),
        "tpl" => s("smarty", "#cbcb41"),
        "tres" => s("godot", "#cbcb41"),
        "ts" => s("typescript", "#519aba"),
        "tscn" => s("godot", "#a074c4"),
        "ttf" => s("font", "#cc3e44"),
        "twig" => s("twig", "#8dc149"),
        "vala" => s("vala", "#6d8086"),
        "vapi" => s("vala", "#6d8086"),
        "vue" => s("vue", "#8dc149"),
        "wasm" => s("wasm", "#a074c4"),
        "wat" => s("wat", "#a074c4"),
        "wav" => s("audio", "#a074c4"),
        "webm" => s("video", "#f55385"),
        "webp" => s("image", "#a074c4"),
        "wgt" => s("wgt", "#519aba"),
        "woff" => s("font", "#cc3e44"),
        "woff2" => s("font", "#cc3e44"),
        "xhtml" => s("html", "#e37933"),
        "xls" => s("xls", "#8dc149"),
        "xlsx" => s("xls", "#8dc149"),
        "xml" => s("xml", "#e37933"),
        "yaml" => s("yml", "#a074c4"),
        "yml" => s("yml", "#a074c4"),
        "zig" => s("zig", "#e37933"),
        "zip" => s("zip", "#6d8086"),
        "zsh" => s("shell", "#8dc149"),
        _ => s("default", "#d4d7d6"),
    }
}

/// Small tinted SVG for a Files list row (Seti glyph + Seti color).
pub fn entry_icon(name: &str, is_dir: bool) -> impl IntoElement {
    let Icon { svg: glyph, color } = icon_for(name, is_dir);
    let path = format!("icons/files/{}.svg", glyph);
    svg()
        .path(SharedString::from(path))
        .size(px(ICON))
        .flex_shrink_0()
        .text_color(color)
}
