// src/bin/gen_ascii.rs
//
// Run with: `cargo run --bin gen_ascii --release`
// Reads:    src/neofetch_source.sh  (upstream neofetch script)
// Writes:   src/ascii_data.rs       (generated Rust source)

use std::collections::HashSet;
use std::fs;

fn main() -> std::io::Result<()> {
    let src = fs::read_to_string("src/neofetch_source.sh")?;
    let out = generate(&src);
    fs::write("src/ascii_data.rs", out)?;
    println!("OK: src/ascii_data.rs generated");
    Ok(())
}

/// Parse the `get_distro_ascii` function from neofetch and emit
/// a Rust `match` with one arm per distro.
fn generate(src: &str) -> String {
    // Extract only the `get_distro_ascii()` function body — everything
    // else in the script is irrelevant.
    let start = src.find("get_distro_ascii()").unwrap();
    let end = src[start..].find("\nmain \"$@\"").unwrap() + start;
    let block = &src[start..end];

    // Each entry: (max_key_length, rust_match_arm).
    // We keep max_key_length so we can sort most-specific-first later.
    let mut arms: Vec<(usize, String)> = Vec::new();

    // Track keys we've already emitted to avoid duplicate match arms.
    let mut seen_keys: HashSet<String> = HashSet::new();

    let mut lines = block.lines().peekable();

    // State for the current case arm.
    let mut current_colors: Vec<u32> = vec![7, 7, 7, 7, 7, 7];
    let mut current_names: Vec<String> = Vec::new();
    let mut in_arm = false;

    while let Some(line) = lines.next() {
        let t = line.trim();

        // Case-arm start: `"Name"*)`, `"A"* | "B"*)`, or `*"Name"*)`.
        if !in_arm && (t.starts_with('"') || t.starts_with('*')) && t.contains(')') {
            if let Some(close) = t.find(')') {
                let pat = &t[..close];
                current_names = parse_names(pat);
                current_colors = vec![7, 7, 7, 7, 7, 7];
                in_arm = true;
            }
            continue;
        }

        if !in_arm {
            continue;
        }

        // Inside an arm: `set_colors N M ...` sets the palette.
        if t.starts_with("set_colors ") {
            let nums: Vec<u32> = t["set_colors ".len()..]
                .split_whitespace()
                .filter_map(|s| s.parse().ok())
                .collect();
            if !nums.is_empty() {
                current_colors = nums;
            }
            continue;
        }

        // Inside an arm: `read -rd '' ascii_data <<'EOF' ... EOF`
        // Everything between the opening and closing EOF is the art.
        if t.contains("<<'EOF'") {
            let mut art = String::new();
            while let Some(l) = lines.next() {
                if l.trim() == "EOF" {
                    break;
                }
                art.push_str(l);
                art.push('\n');
            }
            // Strip the trailing newline we always append.
            if art.ends_with('\n') {
                art.pop();
            }

            // Keep only non-trivial, not-yet-seen keys.
            let mut fresh_names: Vec<String> = Vec::new();
            for name in &current_names {
                if name.len() < 2 {
                    continue;
                }
                if seen_keys.insert(name.clone()) {
                    fresh_names.push(name.clone());
                }
            }

            if !fresh_names.is_empty() {
                // Longest key in this arm — used for sorting.
                let max_len = fresh_names.iter().map(|s| s.len()).max().unwrap_or(0);

                // Build a Rust guard: `n.starts_with("a") || n.starts_with("b") || ...`
                let cond = fresh_names
                    .iter()
                    .map(|k| format!("n.starts_with(\"{}\")", k))
                    .collect::<Vec<_>>()
                    .join(" || ");

                let art_escaped = escape_rust_string(&art);

                // Colors c1..c6 with sane defaults (matching neofetch's `set_colors`).
                let colors = [
                    current_colors.get(0).copied().unwrap_or(7),
                    current_colors.get(1).copied().unwrap_or(7),
                    current_colors.get(2).copied().unwrap_or(4),
                    current_colors.get(3).copied().unwrap_or(1),
                    current_colors.get(4).copied().unwrap_or(6),
                    current_colors.get(5).copied().unwrap_or(5),
                ];

                arms.push((
                    max_len,
                    format!(
                        "        n if {} => Some((\"{}\", [{}, {}, {}, {}, {}, {}])),",
                        cond, art_escaped,
                        colors[0], colors[1], colors[2], colors[3], colors[4], colors[5]
                    ),
                ));
            }

            in_arm = false;
            current_names.clear();
            continue;
        }

        // End of the current case arm.
        if t == ";;" {
            in_arm = false;
            current_names.clear();
        }
    }

    // Sort arms by key length descending so that specific keys
    // (e.g. "linux mint") are tested before generic ones ("linux").
    arms.sort_by(|a, b| b.0.cmp(&a.0));
    let arms_sorted: Vec<String> = arms.into_iter().map(|(_, s)| s).collect();

    format!(
        "// AUTO-GENERATED — do not edit by hand.\n\
         // Generated from src/neofetch_source.sh via `cargo run --bin gen_ascii`.\n\
         \n\
         pub fn get_ascii(os: &str) -> Option<(&'static str, [u8; 6])> {{\n\
         \x20   let os_l = os.to_lowercase();\n\
         \x20   let name = os_l.trim();\n\
         \x20   match name {{\n\
         {}\n\
         \x20       _ => None,\n\
         \x20   }}\n\
         }}\n",
        arms_sorted.join("\n")
    )
}

/// Parse a case pattern like `"Linux Mint"* | "LinuxMint"* | "mint"*`
/// or the bash glob-concat form `"mac"*"_small"`.
///
/// Returns a list of lowercase keys (no `*`, no quotes) that the
/// generated Rust code will use in `starts_with(...)` guards.
fn parse_names(pat: &str) -> Vec<String> {
    let mut out = Vec::new();

    for part in pat.split('|') {
        let p = part.trim();

        // Special case: `"mac"*"_small"`, `"mac"*"_old"`, etc.
        // Multiple quoted fragments separated by `*`. Join them with `_`.
        if p.matches('"').count() >= 2 {
            let mut fragments: Vec<String> = Vec::new();
            let mut current = String::new();
            let mut in_quotes = false;

            for c in p.chars() {
                match c {
                    '"' => {
                        if in_quotes {
                            fragments.push(current.clone());
                            current.clear();
                        }
                        in_quotes = !in_quotes;
                    }
                    _ => {
                        if in_quotes {
                            current.push(c);
                        }
                    }
                }
            }

            fragments.retain(|s| !s.is_empty() && s != "*");
            if !fragments.is_empty() {
                out.push(fragments.join("_").to_lowercase());
                continue;
            }
        }

        // Normal case: single quoted name, optionally followed by `*`.
        let cleaned = p.trim_start_matches('*').trim_end_matches('*').trim();
        let name = cleaned.trim_matches('"').trim();
        if name.is_empty() || name == "*" {
            continue;
        }
        out.push(name.to_lowercase());
    }

    out
}

/// Escape a string so it can be embedded in a Rust string literal.
fn escape_rust_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 16);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            _ => out.push(c),
        }
    }
    out
}
