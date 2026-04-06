use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use regex::Regex;

use crate::parse::source::SourceFile;

static DIRECTIVE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"#\s*(?:rubocop|nitrocop)\s*:\s*(disable|enable|todo)\s+(.+)").unwrap()
});

/// Normalize a cop name from disable comments.
///
/// RuboCop's `DIRECTIVE_COMMENT_REGEXP` uses `COP_NAME_PATTERN` =
/// `([A-Za-z]\w+/)*(?:[A-Za-z]\w+)`, which splits on `/` only — `:` is not
/// part of `\w`.  When a user writes `Department::CopName`, the regex captures
/// only `Department` (the part before `::`), and the `::CopName` suffix falls
/// outside the match.  RuboCop then treats `Department` as a department-level
/// disable that suppresses every cop in that department.
///
/// We replicate this by stripping the `::…` suffix and returning just the
/// department token, so the range is stored under the department key and
/// `is_disabled` matches via the department check.
pub fn normalize_directive_cop_name(name: &str) -> String {
    // Strip trailing `/` — users write `# rubocop:disable Metrics/` to mean
    // the department.  RuboCop treats `Metrics/` identically to `Metrics`.
    let name = name.strip_suffix('/').unwrap_or(name);

    if let Some((dept, _cop)) = name.split_once("::") {
        return dept.to_string();
    }

    // Normalize slash-separated names where the first two segments form a
    // CamelCase department.  Users write `RSpec/Rails/InferredSpecType` in
    // disable comments, but our canonical cop name is `RSpecRails/InferredSpecType`.
    // Also handles department-level `RSpec/Rails` → `RSpecRails`.
    let parts: Vec<&str> = name.split('/').collect();
    if parts.len() >= 2 {
        let combined = format!("{}{}", parts[0], parts[1]);
        if THREE_PART_DEPARTMENTS.contains(&combined.as_str()) {
            return if parts.len() == 2 {
                combined
            } else {
                format!("{}/{}", combined, parts[2..].join("/"))
            };
        }
    }

    name.to_string()
}

/// Known CamelCase departments that users may write as two slash-separated
/// segments in directive comments (e.g. `RSpec/Rails` → `RSpecRails`).
const THREE_PART_DEPARTMENTS: &[&str] = &["RSpecRails"];

/// Legacy directive aliases derived from obsoletion.yml.
///
/// Maps new cop name -> old cop names when RuboCop still treats the old name as
/// suppressing the new cop in inline directives. Only includes renames where
/// the short name (after the `/`) stayed the same, because RuboCop's
/// `Registry.qualified_cop_name` resolves unregistered names by short-name
/// lookup in the global registry:
/// - moved cops whose short name stayed the same
///   (`Lint/Eval` -> `Security/Eval`, `Metrics/LineLength` -> `Layout/LineLength`)
///
/// Renames that changed the short name are excluded even within the same
/// department (`Naming/PredicateName` -> `Naming/PredicatePrefix`), because
/// the old short name `PredicateName` won't match any registered cop.
static DIRECTIVE_LEGACY_ALIASES: LazyLock<HashMap<String, Vec<String>>> =
    LazyLock::new(|| build_directive_legacy_aliases(&crate::linter::RENAMED_COPS));

/// A single disable directive entry (one cop name from a `# rubocop:disable` comment).
#[derive(Debug, Clone)]
pub struct DisableDirective {
    /// The cop/department/all name exactly as written in the comment.
    pub cop_name: String,
    /// RuboCop-style qualified key used internally for matching.
    key: String,
    /// 1-indexed line number of the directive comment.
    pub line: usize,
    /// 0-indexed column of the `#` starting the comment.
    pub column: usize,
    /// Whether this directive is inline (code before the `#` on the same line).
    pub is_inline: bool,
    /// The line range this directive covers (start_line, end_line) inclusive, 1-indexed.
    pub range: (usize, usize),
    /// Whether this directive actually suppressed at least one diagnostic.
    pub used: bool,
}

/// Tracks line ranges where cops are disabled via inline comments.
///
/// Supports `# rubocop:disable`, `# rubocop:enable`, `# rubocop:todo`,
/// and the `# nitrocop:` equivalents.
pub struct DisabledRanges {
    /// Map from cop name (e.g. "Layout/LineLength"), department (e.g. "Metrics"),
    /// or "all" to disabled line ranges. Each range is (start_line, end_line)
    /// inclusive, 1-indexed.
    ranges: HashMap<String, Vec<(usize, usize)>>,
    /// If true, no directives were found — skip filtering entirely.
    empty: bool,
    /// All disable directives found, for redundancy checking.
    directives: Vec<DisableDirective>,
}

struct DirectiveRegistryInfo {
    registered: HashSet<String>,
    short_to_qualified: HashMap<String, Vec<String>>,
}

impl DirectiveRegistryInfo {
    fn from_registry(registry: &crate::cop::registry::CopRegistry) -> Self {
        let mut registered = HashSet::new();
        let mut short_to_qualified: HashMap<String, Vec<String>> = HashMap::new();

        for name in registry.names() {
            registered.insert(name.to_string());
            if let Some((_, short)) = name.split_once('/') {
                short_to_qualified
                    .entry(short.to_string())
                    .or_default()
                    .push(name.to_string());
            }
        }

        Self {
            registered,
            short_to_qualified,
        }
    }

    fn qualify(&self, name: &str) -> String {
        let badge = DirectiveBadge::parse(name);
        // Mirror RuboCop::Cop::Registry.qualified_cop_name:
        // - if Badge.parse(name) is already a registered qualified badge,
        //   return the original text unchanged
        // - otherwise, only resolve by short name when it maps to exactly one cop
        if self.registered.contains(&badge.registered_name()) {
            return name.to_string();
        }

        match self.short_to_qualified.get(badge.cop_name.as_str()) {
            Some(matches) if matches.len() == 1 => matches[0].clone(),
            _ => name.to_string(),
        }
    }
}

struct DirectiveBadge {
    department: Option<String>,
    cop_name: String,
}

impl DirectiveBadge {
    fn parse(name: &str) -> Self {
        let mut parts = name.split('/').map(camel_case_directive_part);
        let cop_name = parts.next_back().unwrap_or_default();
        let department = {
            let parts: Vec<String> = parts.collect();
            if parts.is_empty() {
                None
            } else {
                Some(parts.join("/"))
            }
        };

        Self {
            department,
            cop_name,
        }
    }

    fn registered_name(&self) -> String {
        match &self.department {
            Some(department) => format!("{department}/{}", self.cop_name),
            None => self.cop_name.clone(),
        }
    }
}

fn camel_case_directive_part(name_part: &str) -> String {
    if name_part == "rspec" {
        return "RSpec".to_string();
    }

    let bytes = name_part.as_bytes();
    let needs_camel_case = bytes.first().is_some_and(u8::is_ascii_lowercase)
        || bytes
            .windows(2)
            .any(|window| window[0] == b'_' && window[1].is_ascii_lowercase());
    if !needs_camel_case {
        return name_part.to_string();
    }

    let mut result = String::with_capacity(name_part.len());
    let mut chars = name_part.chars().peekable();
    let mut at_start = true;

    while let Some(ch) = chars.next() {
        if at_start && ch.is_ascii_lowercase() {
            result.push(ch.to_ascii_uppercase());
            at_start = false;
            continue;
        }

        if ch == '_' {
            if let Some(next) = chars.next_if(|next| next.is_ascii_lowercase()) {
                result.push(next.to_ascii_uppercase());
            } else {
                result.push(ch);
            }
            at_start = false;
            continue;
        }

        result.push(ch);
        at_start = false;
    }

    result
}

impl DisabledRanges {
    pub fn from_comments(
        source: &SourceFile,
        parse_result: &ruby_prism::ParseResult<'_>,
        registry: &crate::cop::registry::CopRegistry,
    ) -> Self {
        let mut ranges: HashMap<String, Vec<(usize, usize)>> = HashMap::new();
        // Track open block disables: cop_name -> (start_line, column, directive_index)
        let mut open_disables: HashMap<String, (usize, usize, usize)> = HashMap::new();
        let mut found_any = false;
        let mut directives: Vec<DisableDirective> = Vec::new();
        let registry_info = DirectiveRegistryInfo::from_registry(registry);

        let lines: Vec<&[u8]> = source.lines().collect();

        for comment in parse_result.comments() {
            let loc = comment.location();
            let comment_bytes = &source.as_bytes()[loc.start_offset()..loc.end_offset()];
            let Ok(comment_str) = std::str::from_utf8(comment_bytes) else {
                continue;
            };

            let Some(caps) = DIRECTIVE_RE.captures(comment_str) else {
                continue;
            };

            let (line, col) = source.offset_to_line_col(loc.start_offset());

            // Determine if inline: check if there's non-whitespace before the comment
            let is_inline = if line >= 1 && line <= lines.len() {
                let line_bytes = lines[line - 1];
                let before_comment = &line_bytes[..col.min(line_bytes.len())];
                before_comment.iter().any(|b| !b.is_ascii_whitespace())
            } else {
                false
            };

            // Reject YARD doc nested comments like `#   # rubocop:disable all`
            // where Prism reports the entire line as one comment token.
            // The text before the directive match is only `#` + whitespace.
            // Only reject on standalone comment lines — inline comments with
            // double-# (e.g., `rescue Exception # # rubocop:disable Cop`) are
            // legitimate directives.
            let match_start = caps.get(0).unwrap().start();
            if match_start > 0 && !is_inline {
                let prefix = &comment_str[..match_start];
                if prefix.bytes().all(|b| b == b'#' || b == b' ' || b == b'\t') {
                    continue;
                }
            }

            found_any = true;

            let action = &caps[1];
            let cop_list_raw = &caps[2];

            // Strip trailing comment marker (-- reason)
            let cop_list = match cop_list_raw.find("--") {
                Some(idx) => &cop_list_raw[..idx],
                None => cop_list_raw,
            };

            let cop_names: Vec<&str> = cop_list
                .split(',')
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(|s| {
                    // Extract just the cop name, ignoring trailing free-text comments.
                    // Cop names are: "all", "Department", or "Department/CopName".
                    // If there's a space after the cop name, the rest is a comment.
                    let s = match s.find(' ') {
                        Some(idx) => &s[..idx],
                        None => s,
                    };
                    // Strip parenthesized annotations like (RuboCop) in
                    // `# rubocop:disable Metrics/BlockLength(RuboCop)`.
                    // RuboCop accepts this syntax and ignores the annotation.
                    match s.find('(') {
                        Some(idx) => &s[..idx],
                        None => s,
                    }
                })
                .filter(|s| !s.is_empty())
                .map(|s| {
                    // Handle wildcard department patterns like `Style/*` before
                    // trimming, since `*` would be stripped as non-identifier.
                    // Normalize to just the department name.
                    if let Some(dept) = s.strip_suffix("/*") {
                        return dept;
                    }
                    // Strip trailing non-identifier chars (e.g., trailing `?` in
                    // `Naming/PredicatePrefix?`). RuboCop's regex stops at
                    // `[A-Za-z]\w+(/[A-Za-z]\w+)*` so trailing punctuation is ignored.
                    s.trim_end_matches(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '/')
                })
                .filter(|s| !s.is_empty())
                .collect();

            match action {
                "disable" | "todo" => {
                    for &cop in &cop_names {
                        // Normalize Department::CopName -> Department/CopName
                        let cop = normalize_directive_cop_name(cop);
                        let key = registry_info.qualify(cop.as_str());
                        let key = key.as_str();
                        if is_inline {
                            let range = (line, line);
                            ranges.entry(key.to_string()).or_default().push(range);
                            directives.push(DisableDirective {
                                cop_name: cop.to_string(),
                                key: key.to_string(),
                                line,
                                column: col,
                                is_inline: true,
                                range,
                                used: false,
                            });
                        } else {
                            // Close any existing open disable for the same cop
                            // before opening a new one. This handles duplicate
                            // `# rubocop:disable Cop` without an intervening
                            // `# rubocop:enable Cop`.
                            if let Some((prev_start, _prev_col, prev_idx)) =
                                open_disables.remove(key)
                            {
                                let range = (prev_start, line);
                                ranges.entry(key.to_string()).or_default().push(range);
                                if prev_idx < directives.len() {
                                    directives[prev_idx].range = range;
                                }
                            }
                            let directive_idx = directives.len();
                            directives.push(DisableDirective {
                                cop_name: cop.to_string(),
                                key: key.to_string(),
                                line,
                                column: col,
                                is_inline: false,
                                range: (line, usize::MAX), // placeholder, updated on enable/EOF
                                used: false,
                            });
                            open_disables.insert(key.to_string(), (line, col, directive_idx));
                        }
                    }
                }
                "enable" => {
                    // Inline enables (trailing comment on a code line) do NOT
                    // close block disables in RuboCop. Only standalone enables
                    // (on their own line) close open block disables. An inline
                    // enable is effectively a no-op.
                    if is_inline {
                        continue;
                    }
                    for &cop in &cop_names {
                        // Normalize Department::CopName -> Department/CopName
                        let cop = normalize_directive_cop_name(cop);
                        let key = registry_info.qualify(cop.as_str());
                        let key = key.as_str();
                        if key == "all" {
                            // `# rubocop:enable all` closes ALL open disables,
                            // not just a disable for the literal string "all".
                            for (open_cop, (start_line, _col, directive_idx)) in
                                open_disables.drain()
                            {
                                let range = (start_line, line);
                                ranges.entry(open_cop).or_default().push(range);
                                if directive_idx < directives.len() {
                                    directives[directive_idx].range = range;
                                }
                            }
                        } else if let Some(dept) = key.strip_suffix("/*").or_else(|| {
                            // A bare department name (no `/`) also closes all cops
                            // in that department.
                            if !key.contains('/') { Some(key) } else { None }
                        }) {
                            // `# rubocop:enable Department` closes the department
                            // disable AND any individual cop disables in that dept.
                            // First close exact match (department name itself)
                            if let Some((start_line, _col, directive_idx)) =
                                open_disables.remove(dept)
                            {
                                let range = (start_line, line);
                                ranges.entry(dept.to_string()).or_default().push(range);
                                if directive_idx < directives.len() {
                                    directives[directive_idx].range = range;
                                }
                            }
                            // Also close any individual cops in that department
                            let dept_prefix = format!("{dept}/");
                            let matching_cops: Vec<String> = open_disables
                                .keys()
                                .filter(|k| k.starts_with(&dept_prefix))
                                .cloned()
                                .collect();
                            for open_cop in matching_cops {
                                if let Some((start_line, _col, directive_idx)) =
                                    open_disables.remove(&open_cop)
                                {
                                    let range = (start_line, line);
                                    ranges.entry(open_cop).or_default().push(range);
                                    if directive_idx < directives.len() {
                                        directives[directive_idx].range = range;
                                    }
                                }
                            }
                        } else if let Some((start_line, _col, directive_idx)) =
                            open_disables.remove(key)
                        {
                            let range = (start_line, line);
                            ranges.entry(key.to_string()).or_default().push(range);
                            // Update the directive's range
                            if directive_idx < directives.len() {
                                directives[directive_idx].range = range;
                            }
                        }
                        // Orphaned enable without prior disable: ignore
                    }
                }
                _ => {}
            }
        }

        // Close any remaining open disables to EOF
        for (cop, (start_line, _col, directive_idx)) in open_disables {
            let range = (start_line, usize::MAX);
            ranges.entry(cop).or_default().push(range);
            if directive_idx < directives.len() {
                directives[directive_idx].range = range;
            }
        }

        DisabledRanges {
            ranges,
            empty: !found_any,
            directives,
        }
    }

    /// Returns true if `cop_name` is disabled at `line`.
    ///
    /// Checks the exact cop name, RuboCop-compatible moved-name aliases,
    /// its department prefix, and "all".
    pub fn is_disabled(&self, cop_name: &str, line: usize) -> bool {
        // Check exact cop name
        if self.check_ranges(cop_name, line) {
            return true;
        }

        // Check legacy aliases that RuboCop still honors in directive comments.
        if let Some(aliases) = DIRECTIVE_LEGACY_ALIASES.get(cop_name) {
            for alias in aliases {
                if self.check_ranges(alias, line) {
                    return true;
                }
            }
        }

        // Check department name (e.g., "Layout" for "Layout/LineLength")
        if let Some(dept) = cop_name.split('/').next() {
            if dept != cop_name && self.check_ranges(dept, line) {
                return true;
            }
        }

        // Check "all"
        self.check_ranges("all", line)
    }

    /// Check if a diagnostic is disabled AND mark the matching directive(s) as used.
    ///
    /// Returns true if the diagnostic should be suppressed.
    pub fn check_and_mark_used(&mut self, cop_name: &str, line: usize) -> bool {
        let mut suppressed = false;

        // Check exact cop name
        if self.check_ranges(cop_name, line) {
            self.mark_directives_used(cop_name, line);
            suppressed = true;
        }

        // Check legacy aliases that RuboCop still honors in directive comments.
        if let Some(aliases) = DIRECTIVE_LEGACY_ALIASES.get(cop_name) {
            for alias in aliases {
                if self.check_ranges(alias, line) {
                    self.mark_directives_used(alias, line);
                    suppressed = true;
                }
            }
        }

        // Check department name (e.g., "Layout" for "Layout/LineLength")
        if let Some(dept) = cop_name.split('/').next() {
            if dept != cop_name && self.check_ranges(dept, line) {
                self.mark_directives_used(dept, line);
                suppressed = true;
            }
        }

        // Check "all"
        if self.check_ranges("all", line) {
            self.mark_directives_used("all", line);
            suppressed = true;
        }

        suppressed
    }

    /// Mark all directives with the given key that cover the given line as used.
    fn mark_directives_used(&mut self, key: &str, line: usize) {
        for directive in &mut self.directives {
            if directive.key == key && line >= directive.range.0 && line <= directive.range.1 {
                directive.used = true;
            }
        }
    }

    /// Return all unused disable directives (those that didn't suppress any diagnostic).
    pub fn unused_directives(&self) -> impl Iterator<Item = &DisableDirective> {
        self.directives.iter().filter(|d| !d.used)
    }

    pub fn is_empty(&self) -> bool {
        self.empty
    }

    /// Returns true if there are any disable directives (used for redundancy checking).
    pub fn has_directives(&self) -> bool {
        !self.directives.is_empty()
    }

    pub(crate) fn check_ranges(&self, key: &str, line: usize) -> bool {
        if let Some(ranges) = self.ranges.get(key) {
            for &(start, end) in ranges {
                if line >= start && line <= end {
                    return true;
                }
            }
        }
        false
    }
}

fn build_directive_legacy_aliases(
    renamed_cops: &HashMap<String, String>,
) -> HashMap<String, Vec<String>> {
    let mut aliases = HashMap::new();

    for (old_name, new_name) in renamed_cops {
        let Some((_, old_short)) = old_name.split_once('/') else {
            continue;
        };
        let Some((_, new_short)) = new_name.as_str().split_once('/') else {
            continue;
        };
        let same_short_name = old_short.eq_ignore_ascii_case(new_short);
        if !same_short_name {
            continue;
        }

        aliases
            .entry(new_name.clone())
            .or_insert_with(Vec::new)
            .push(old_name.clone());
    }

    aliases
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::source::SourceFile;

    fn disabled_ranges(src: &str) -> DisabledRanges {
        let source = SourceFile::from_bytes("test.rb", src.as_bytes().to_vec());
        let parse_result = crate::parse::parse_source(source.as_bytes());
        let registry = crate::cop::registry::CopRegistry::default_registry();
        DisabledRanges::from_comments(&source, &parse_result, &registry)
    }

    #[test]
    fn single_line_inline_disable() {
        let dr = disabled_ranges("x = 1 # rubocop:disable Foo/Bar\ny = 2\n");
        assert!(!dr.is_empty());
        assert!(dr.is_disabled("Foo/Bar", 1));
        assert!(!dr.is_disabled("Foo/Bar", 2));
    }

    #[test]
    fn block_disable_enable() {
        let src = "# rubocop:disable Foo/Bar\nx = 1\ny = 2\n# rubocop:enable Foo/Bar\nz = 3\n";
        let dr = disabled_ranges(src);
        assert!(dr.is_disabled("Foo/Bar", 1));
        assert!(dr.is_disabled("Foo/Bar", 2));
        assert!(dr.is_disabled("Foo/Bar", 3));
        assert!(dr.is_disabled("Foo/Bar", 4));
        assert!(!dr.is_disabled("Foo/Bar", 5));
    }

    #[test]
    fn unterminated_disable() {
        let src = "# rubocop:disable Foo/Bar\nx = 1\ny = 2\n";
        let dr = disabled_ranges(src);
        assert!(dr.is_disabled("Foo/Bar", 1));
        assert!(dr.is_disabled("Foo/Bar", 2));
        assert!(dr.is_disabled("Foo/Bar", 3));
        assert!(dr.is_disabled("Foo/Bar", 999_999));
    }

    #[test]
    fn multiple_cops() {
        let src = "x = 1 # rubocop:disable Foo/Bar, Baz/Qux\ny = 2\n";
        let dr = disabled_ranges(src);
        assert!(dr.is_disabled("Foo/Bar", 1));
        assert!(dr.is_disabled("Baz/Qux", 1));
        assert!(!dr.is_disabled("Foo/Bar", 2));
        assert!(!dr.is_disabled("Baz/Qux", 2));
    }

    #[test]
    fn department_disable() {
        let src = "# rubocop:disable Metrics\nx = 1\n# rubocop:enable Metrics\ny = 2\n";
        let dr = disabled_ranges(src);
        assert!(dr.is_disabled("Metrics/MethodLength", 2));
        assert!(dr.is_disabled("Metrics/AbcSize", 2));
        assert!(!dr.is_disabled("Layout/LineLength", 2));
        // Enable line itself is still in the disabled range
        assert!(dr.is_disabled("Metrics/MethodLength", 3));
        // Line after enable is no longer disabled
        assert!(!dr.is_disabled("Metrics/MethodLength", 4));
    }

    #[test]
    fn department_disable_trailing_slash() {
        // `# rubocop:disable Metrics/` (with trailing slash) should work
        // identically to `# rubocop:disable Metrics`.
        let src = "# rubocop:disable Metrics/\nx = 1\n# rubocop:enable Metrics/\ny = 2\n";
        let dr = disabled_ranges(src);
        assert!(
            dr.is_disabled("Metrics/MethodLength", 2),
            "Metrics/ should disable Metrics/MethodLength"
        );
        assert!(
            dr.is_disabled("Metrics/CyclomaticComplexity", 2),
            "Metrics/ should disable Metrics/CyclomaticComplexity"
        );
        assert!(!dr.is_disabled("Layout/LineLength", 2));
        assert!(!dr.is_disabled("Metrics/MethodLength", 4));
    }

    #[test]
    fn department_disable_trailing_slash_inline() {
        // Inline variant: `x = 1 # rubocop:disable Metrics/ -- reason`
        let src = "x = 1 # rubocop:disable Metrics/ -- It's more readable this way\ny = 2\n";
        let dr = disabled_ranges(src);
        assert!(
            dr.is_disabled("Metrics/PerceivedComplexity", 1),
            "inline Metrics/ should disable Metrics cops on same line"
        );
        assert!(!dr.is_disabled("Metrics/PerceivedComplexity", 2));
    }

    #[test]
    fn short_cop_name_disable() {
        let src = "# rubocop:disable MethodLength\nx = 1\n# rubocop:enable MethodLength\ny = 2\n";
        let dr = disabled_ranges(src);
        assert!(dr.is_disabled("Metrics/MethodLength", 2));
        assert!(dr.is_disabled("Metrics/MethodLength", 3));
        assert!(!dr.is_disabled("Metrics/MethodLength", 4));
    }

    #[test]
    fn three_part_cop_name_disable() {
        // `RSpec/Rails/InferredSpecType` should disable `RSpecRails/InferredSpecType`
        let src = "# rubocop:disable RSpec/Rails/InferredSpecType\nx = 1\n# rubocop:enable RSpec/Rails/InferredSpecType\ny = 2\n";
        let dr = disabled_ranges(src);
        assert!(dr.is_disabled("RSpecRails/InferredSpecType", 2));
        assert!(!dr.is_disabled("RSpecRails/InferredSpecType", 4));
    }

    #[test]
    fn three_part_department_disable() {
        // `RSpec/Rails` should disable all `RSpecRails/*` cops
        let src = "# rubocop:disable RSpec/Rails\nx = 1\n# rubocop:enable RSpec/Rails\ny = 2\n";
        let dr = disabled_ranges(src);
        assert!(dr.is_disabled("RSpecRails/InferredSpecType", 2));
        assert!(!dr.is_disabled("RSpecRails/InferredSpecType", 4));
    }

    #[test]
    fn disable_all() {
        let src = "# rubocop:disable all\nx = 1\n# rubocop:enable all\ny = 2\n";
        let dr = disabled_ranges(src);
        assert!(dr.is_disabled("Layout/LineLength", 2));
        assert!(dr.is_disabled("Style/Foo", 2));
        assert!(!dr.is_disabled("Layout/LineLength", 4));
    }

    #[test]
    fn nitrocop_alias() {
        let dr = disabled_ranges("x = 1 # nitrocop:disable Foo/Bar\ny = 2\n");
        assert!(dr.is_disabled("Foo/Bar", 1));
        assert!(!dr.is_disabled("Foo/Bar", 2));
    }

    #[test]
    fn standard_alias_not_supported() {
        let dr = disabled_ranges("x = 1 # standard:disable Foo/Bar\ny = 2\n");
        assert!(!dr.is_disabled("Foo/Bar", 1));
        assert!(!dr.is_disabled("Foo/Bar", 2));
    }

    #[test]
    fn todo_acts_as_disable() {
        let src = "# rubocop:todo Foo/Bar\nx = 1\n# rubocop:enable Foo/Bar\ny = 2\n";
        let dr = disabled_ranges(src);
        assert!(dr.is_disabled("Foo/Bar", 1));
        assert!(dr.is_disabled("Foo/Bar", 2));
        assert!(dr.is_disabled("Foo/Bar", 3));
        assert!(!dr.is_disabled("Foo/Bar", 4));
    }

    #[test]
    fn trailing_comment_marker() {
        let src = "x = 1 # rubocop:disable Foo/Bar -- reason why\ny = 2\n";
        let dr = disabled_ranges(src);
        assert!(dr.is_disabled("Foo/Bar", 1));
        assert!(!dr.is_disabled("Foo/Bar", 2));
        // "-- reason why" should not be parsed as a cop name
        assert!(!dr.is_disabled("-- reason why", 1));
    }

    #[test]
    fn no_space_after_hash() {
        let dr = disabled_ranges("x = 1 #rubocop:disable Foo/Bar\ny = 2\n");
        assert!(dr.is_disabled("Foo/Bar", 1));
    }

    #[test]
    fn extra_whitespace() {
        let dr = disabled_ranges("x = 1 # rubocop : disable Foo/Bar\ny = 2\n");
        assert!(dr.is_disabled("Foo/Bar", 1));
    }

    #[test]
    fn no_directives() {
        let dr = disabled_ranges("x = 1\ny = 2\n");
        assert!(dr.is_empty());
        assert!(!dr.is_disabled("Foo/Bar", 1));
    }

    #[test]
    fn parenthesized_annotation_stripped() {
        // RuboCop accepts `# rubocop:disable Cop(annotation)` syntax
        let src = "# rubocop:disable Metrics/BlockLength(RuboCop)\nx = 1\n# rubocop:enable Metrics/BlockLength\ny = 2\n";
        let dr = disabled_ranges(src);
        assert!(dr.is_disabled("Metrics/BlockLength", 1));
        assert!(dr.is_disabled("Metrics/BlockLength", 2));
        assert!(dr.is_disabled("Metrics/BlockLength", 3));
        assert!(!dr.is_disabled("Metrics/BlockLength", 4));
    }

    #[test]
    fn enable_all_closes_individual_cop_disables() {
        // `# rubocop:disable Layout/EndAlignment` followed by `# rubocop:enable all`
        // should close the Layout/EndAlignment disable.
        let src = "    # rubocop:disable Layout/IndentationWidth, Layout/EndAlignment\n\
                   x = if true\n\
                     1\n\
                   end\n\
                   # rubocop:enable all\n\
                   y = if true\n\
                     2\n\
                   end\n";
        let dr = disabled_ranges(src);
        // Line 2-4 should be disabled for Layout/EndAlignment (within disable-enable block)
        assert!(
            dr.is_disabled("Layout/EndAlignment", 2),
            "Layout/EndAlignment should be disabled at line 2 (before enable all)"
        );
        // Line 6-8 should NOT be disabled (after `# rubocop:enable all`)
        assert!(
            !dr.is_disabled("Layout/EndAlignment", 6),
            "Layout/EndAlignment should NOT be disabled at line 6 (after enable all)"
        );
        assert!(
            !dr.is_disabled("Layout/EndAlignment", 8),
            "Layout/EndAlignment should NOT be disabled at line 8 (after enable all)"
        );
        // Layout/IndentationWidth should also be re-enabled
        assert!(
            !dr.is_disabled("Layout/IndentationWidth", 6),
            "Layout/IndentationWidth should NOT be disabled at line 6 (after enable all)"
        );
    }

    #[test]
    fn enable_all_after_heredoc_with_interpolation() {
        // Reproduce the exact pattern from rage-rb: disable individual cops,
        // then use heredoc with #{if...end} interpolation, then enable all.
        let src = concat!(
            "class Foo\n",
            "  class << self\n",
            "    # rubocop:disable Layout/IndentationWidth, Layout/EndAlignment\n", // line 3
            "    def foo(action)\n",
            "      class_eval <<~RUBY, __FILE__, __LINE__ + 1\n",
            "        def run_#{action}\n", // line 6 - interpolation
            "          #{if true\n",       // line 7 - if inside interpolation
            "            <<~RUBY\n",       // line 8 - nested heredoc
            "              hello\n",
            "            RUBY\n", // line 10
            "          end}\n",   // line 11
            "        end\n",
            "      RUBY\n", // line 13
            "    end\n",
            "    # rubocop:enable all\n", // line 15
            "  end\n",
            "\n",
            "  def render\n", // line 18
            "    y = if true\n",
            "      2\n",
            "    end\n", // line 21 - should be flagged
            "  end\n",
            "end\n",
        );
        let dr = disabled_ranges(src);
        // Line 7 should be disabled
        assert!(
            dr.is_disabled("Layout/EndAlignment", 7),
            "Layout/EndAlignment should be disabled at line 7"
        );
        // Line 21 (after enable all) should NOT be disabled
        assert!(
            !dr.is_disabled("Layout/EndAlignment", 21),
            "Layout/EndAlignment should NOT be disabled at line 21 (after enable all)"
        );
    }

    #[test]
    fn enable_all_closes_individual_cop_disables_exact_format() {
        // Exact format from rage-rb corpus file
        let src = "    # rubocop:disable Layout/IndentationWidth, Layout/EndAlignment, Layout/HeredocIndentation\n\
                   x = if true\n\
                     1\n\
                   end\n\
                   y = if true\n\
                     2\n\
                   end\n\
                       # rubocop:enable all\n\
                   z = if true\n\
                     3\n\
                   end\n";
        let dr = disabled_ranges(src);
        // Line 2 should be disabled
        assert!(
            dr.is_disabled("Layout/EndAlignment", 2),
            "Layout/EndAlignment should be disabled at line 2"
        );
        // Line 9 (after enable all) should NOT be disabled
        assert!(
            !dr.is_disabled("Layout/EndAlignment", 9),
            "Layout/EndAlignment should NOT be disabled at line 9 (after enable all)"
        );
    }

    #[test]
    fn enable_all_closes_department_disables() {
        // `# rubocop:disable Layout` followed by `# rubocop:enable all`
        let src = "# rubocop:disable Layout\nx = 1\n# rubocop:enable all\ny = 2\n";
        let dr = disabled_ranges(src);
        assert!(
            dr.is_disabled("Layout/EndAlignment", 2),
            "Layout department should be disabled before enable all"
        );
        assert!(
            !dr.is_disabled("Layout/EndAlignment", 4),
            "Layout department should NOT be disabled after enable all"
        );
    }

    #[test]
    fn orphaned_enable_ignored() {
        let dr = disabled_ranges("# rubocop:enable Foo/Bar\nx = 1\n");
        assert!(!dr.is_disabled("Foo/Bar", 1));
        assert!(!dr.is_disabled("Foo/Bar", 2));
    }

    #[test]
    fn inline_enable_does_not_close_block_disable() {
        // RuboCop: inline `# rubocop:enable` (trailing comment on a code line)
        // does NOT close a block `# rubocop:disable`. The block disable stays
        // open to EOF. Only standalone enables close block disables.
        let src = "# rubocop:disable Foo/Bar\nx = 1\nend # rubocop:enable Foo/Bar\ny = 2\nz = 3\n";
        let dr = disabled_ranges(src);
        assert!(
            dr.is_disabled("Foo/Bar", 1),
            "block disable should cover line 1"
        );
        assert!(
            dr.is_disabled("Foo/Bar", 2),
            "block disable should cover line 2"
        );
        assert!(
            dr.is_disabled("Foo/Bar", 3),
            "block disable should cover the inline enable line"
        );
        assert!(
            dr.is_disabled("Foo/Bar", 4),
            "block disable should extend past inline enable (to EOF)"
        );
        assert!(
            dr.is_disabled("Foo/Bar", 5),
            "block disable should extend to EOF"
        );
    }

    #[test]
    fn standalone_enable_does_close_block_disable() {
        // Standalone `# rubocop:enable` on its own line DOES close a block disable.
        let src = "# rubocop:disable Foo/Bar\nx = 1\n# rubocop:enable Foo/Bar\ny = 2\n";
        let dr = disabled_ranges(src);
        assert!(dr.is_disabled("Foo/Bar", 1));
        assert!(dr.is_disabled("Foo/Bar", 2));
        assert!(dr.is_disabled("Foo/Bar", 3)); // enable line itself is still in range
        assert!(!dr.is_disabled("Foo/Bar", 4)); // after enable, no longer disabled
    }

    #[test]
    fn inline_disable_only_affects_that_line() {
        let src = "a = 1 # rubocop:disable Layout/LineLength\nb = 2\nc = 3\n";
        let dr = disabled_ranges(src);
        assert!(dr.is_disabled("Layout/LineLength", 1));
        assert!(!dr.is_disabled("Layout/LineLength", 2));
        assert!(!dr.is_disabled("Layout/LineLength", 3));
    }

    #[test]
    fn standalone_disable_is_range() {
        // A disable on its own line (no code before it) starts a range
        let src = "  # rubocop:disable Layout/LineLength\nb = 2\nc = 3\n  # rubocop:enable Layout/LineLength\nd = 4\n";
        let dr = disabled_ranges(src);
        assert!(dr.is_disabled("Layout/LineLength", 1));
        assert!(dr.is_disabled("Layout/LineLength", 2));
        assert!(dr.is_disabled("Layout/LineLength", 3));
        assert!(dr.is_disabled("Layout/LineLength", 4));
        assert!(!dr.is_disabled("Layout/LineLength", 5));
    }

    #[test]
    fn duplicate_disable_without_enable() {
        // Two disable comments for the same cop without an intervening enable.
        // The first disable should cover lines 1-5, the second covers lines 5+.
        let src = "# rubocop:disable Foo/Bar\nx = 1\nx = 2\nx = 3\n# rubocop:disable Foo/Bar\nx = 4\nx = 5\n";
        let dr = disabled_ranges(src);
        // Lines 1-4 are covered by the first disable (closed at line 5)
        assert!(dr.is_disabled("Foo/Bar", 1));
        assert!(dr.is_disabled("Foo/Bar", 2));
        assert!(dr.is_disabled("Foo/Bar", 3));
        assert!(dr.is_disabled("Foo/Bar", 4));
        // Lines 5+ are covered by the second disable (open to EOF)
        assert!(dr.is_disabled("Foo/Bar", 5));
        assert!(dr.is_disabled("Foo/Bar", 6));
        assert!(dr.is_disabled("Foo/Bar", 7));
    }

    // --- check_and_mark_used tests ---

    #[test]
    fn check_and_mark_used_marks_directive() {
        let mut dr = disabled_ranges("x = 1 # rubocop:disable Foo/Bar\ny = 2\n");
        assert!(dr.check_and_mark_used("Foo/Bar", 1));
        assert!(!dr.check_and_mark_used("Foo/Bar", 2));
        let unused: Vec<_> = dr.unused_directives().collect();
        assert!(unused.is_empty(), "directive should be marked used");
    }

    #[test]
    fn unused_directive_reported() {
        let dr = disabled_ranges("x = 1 # rubocop:disable Foo/Bar\ny = 2\n");
        // Never call check_and_mark_used -> directive stays unused
        let unused: Vec<_> = dr.unused_directives().collect();
        assert_eq!(unused.len(), 1);
        assert_eq!(unused[0].cop_name, "Foo/Bar");
        assert_eq!(unused[0].line, 1);
    }

    #[test]
    fn department_disable_marked_used() {
        let mut dr =
            disabled_ranges("# rubocop:disable Metrics\nx = 1\n# rubocop:enable Metrics\ny = 2\n");
        assert!(dr.check_and_mark_used("Metrics/MethodLength", 2));
        let unused: Vec<_> = dr.unused_directives().collect();
        assert!(
            unused.is_empty(),
            "department directive should be marked used"
        );
    }

    #[test]
    fn short_cop_name_marked_used() {
        let mut dr = disabled_ranges("x = 1 # rubocop:disable MethodLength\ny = 2\n");
        assert!(dr.check_and_mark_used("Metrics/MethodLength", 1));
        let unused: Vec<_> = dr.unused_directives().collect();
        assert!(
            unused.is_empty(),
            "short cop directive should be marked used"
        );
    }

    #[test]
    fn all_disable_marked_used() {
        let mut dr = disabled_ranges("# rubocop:disable all\nx = 1\n# rubocop:enable all\ny = 2\n");
        assert!(dr.check_and_mark_used("Style/Foo", 2));
        let unused: Vec<_> = dr.unused_directives().collect();
        assert!(unused.is_empty(), "all directive should be marked used");
    }

    #[test]
    fn block_directive_unused() {
        let dr = disabled_ranges(
            "# rubocop:disable Foo/Bar\nx = 1\ny = 2\n# rubocop:enable Foo/Bar\nz = 3\n",
        );
        // No diagnostics suppressed
        let unused: Vec<_> = dr.unused_directives().collect();
        assert_eq!(unused.len(), 1);
        assert_eq!(unused[0].cop_name, "Foo/Bar");
        assert_eq!(unused[0].line, 1);
        assert!(!unused[0].is_inline);
    }

    #[test]
    fn multiple_cops_one_used_one_not() {
        let mut dr = disabled_ranges("x = 1 # rubocop:disable Foo/Bar, Baz/Qux\ny = 2\n");
        assert!(dr.check_and_mark_used("Foo/Bar", 1));
        let unused: Vec<_> = dr.unused_directives().collect();
        assert_eq!(unused.len(), 1);
        assert_eq!(unused[0].cop_name, "Baz/Qux");
    }

    #[test]
    fn trailing_non_identifier_chars_stripped() {
        // A trailing `?` on the cop name should be stripped so it matches
        let dr = disabled_ranges("x = 1 # rubocop:disable Naming/PredicatePrefix?\ny = 2\n");
        assert!(
            dr.is_disabled("Naming/PredicatePrefix", 1),
            "trailing ? should be stripped"
        );
        assert!(!dr.is_disabled("Naming/PredicatePrefix?", 1));
    }

    #[test]
    fn wrong_namespace_resolves_single_matching_cop() {
        let src = "# rubocop:disable Rspec/AnyInstance\nx = 1\n# rubocop:enable Rspec/AnyInstance\ny = 2\n";
        let dr = disabled_ranges(src);
        assert!(
            dr.is_disabled("RSpec/AnyInstance", 2),
            "wrong namespace should resolve when the short name is unique"
        );
        assert!(
            !dr.is_disabled("RSpec/AnyInstance", 4),
            "after enable, should not be disabled"
        );
    }

    #[test]
    fn mis_cased_qualified_name_does_not_resolve() {
        let dr = disabled_ranges("x = 1 # rubocop:disable Metrics/abcSize\ny = 2\n");
        assert!(
            !dr.is_disabled("Metrics/AbcSize", 1),
            "mis-cased qualified name should stay raw and not suppress the real cop"
        );
    }

    #[test]
    fn lowercase_department_name_does_not_resolve() {
        let dr = disabled_ranges("x = 1 # rubocop:disable metrics\ny = 2\n");
        assert!(
            !dr.is_disabled("Metrics/AbcSize", 1),
            "lowercase department name should not disable the real department"
        );
    }

    #[test]
    fn camel_cased_short_name_resolves() {
        let dr = disabled_ranges("x = 1 # rubocop:disable abcSize\ny = 2\n");
        assert!(
            dr.is_disabled("Metrics/AbcSize", 1),
            "short names should qualify when Badge.parse resolves them to one cop"
        );
    }

    #[test]
    fn lowercase_multiword_short_name_does_not_resolve() {
        let dr = disabled_ranges("x = 1 # rubocop:disable stringliterals\ny = 2\n");
        assert!(
            !dr.is_disabled("Style/StringLiterals", 1),
            "Badge.parse should not fully camelize all-lowercase multiword short names"
        );
    }

    #[test]
    fn ambiguous_short_name_does_not_resolve() {
        let dr = disabled_ranges("x = 1 # rubocop:disable SafeNavigation\ny = 2\n");
        assert!(
            !dr.is_disabled("Style/SafeNavigation", 1),
            "ambiguous short names should stay raw instead of suppressing one arbitrary cop"
        );
        assert!(
            !dr.is_disabled("Rails/SafeNavigation", 1),
            "ambiguous short names should not suppress the other matching cop either"
        );
    }

    #[test]
    fn moved_cop_same_short_name_resolved() {
        // RuboCop qualifies moved cops by short name when the short name is unchanged.
        let dr = disabled_ranges("x = 1 # rubocop:disable Style/AccessorMethodName\ny = 2\n");
        assert!(
            dr.is_disabled("Naming/AccessorMethodName", 1),
            "moved legacy name should resolve when the short name is unchanged"
        );
    }

    #[test]
    fn same_department_changed_short_name_not_resolved() {
        // Same-department rename where the short name changed should NOT resolve,
        // matching RuboCop behavior: `Registry.qualified_cop_name` resolves by
        // short-name lookup, so `PredicateName` won't find `PredicatePrefix`.
        let dr = disabled_ranges("x = 1 # rubocop:disable Naming/PredicateName\ny = 2\n");
        assert!(
            !dr.is_disabled("Naming/PredicatePrefix", 1),
            "same-department legacy name with changed short name should NOT resolve"
        );
        assert!(
            dr.is_disabled("Naming/PredicateName", 1),
            "the exact legacy name should still be recorded"
        );
    }

    #[test]
    fn moved_cop_same_short_name_block_disable_resolved() {
        let src = "# rubocop:disable Metrics/LineLength\nx = '12345678901234567890'\n# rubocop:enable Metrics/LineLength\ny = 2\n";
        let dr = disabled_ranges(src);
        assert!(
            dr.is_disabled("Layout/LineLength", 2),
            "moved legacy block disable should cover the new cop name"
        );
        assert!(
            !dr.is_disabled("Layout/LineLength", 4),
            "after enable, the new cop name should no longer be disabled"
        );
    }

    #[test]
    fn cross_department_changed_short_name_not_resolved() {
        let dr = disabled_ranges("x = 1 # rubocop:disable Style/OpMethod\ny = 2\n");
        assert!(
            !dr.is_disabled("Naming/BinaryOperatorParameterName", 1),
            "cross-department legacy name with a different short name should not resolve"
        );
        assert!(
            dr.is_disabled("Style/OpMethod", 1),
            "the exact legacy name should still be recorded"
        );
    }

    #[test]
    fn same_department_changed_short_name_not_marked_used() {
        // Same-department rename with changed short name should NOT suppress.
        let mut dr = disabled_ranges("x = 1 # rubocop:disable Naming/PredicateName\ny = 2\n");
        assert!(
            !dr.check_and_mark_used("Naming/PredicatePrefix", 1),
            "changed short name should not suppress new cop"
        );
        let unused: Vec<_> = dr.unused_directives().collect();
        assert_eq!(
            unused.len(),
            1,
            "directive should remain unused since it doesn't match the new cop"
        );
    }

    #[test]
    fn moved_cop_same_short_name_marks_used() {
        let mut dr = disabled_ranges("x = 1 # rubocop:disable Lint/Eval\ny = 2\n");
        assert!(dr.check_and_mark_used("Security/Eval", 1));
        let unused: Vec<_> = dr.unused_directives().collect();
        assert!(
            unused.is_empty(),
            "moved legacy alias directive should be marked used"
        );
    }

    #[test]
    fn yard_nested_comment_not_parsed_as_directive() {
        // YARD doc examples: `#   # rubocop:disable all` should NOT be a real directive
        let src = "# @example\n#   # rubocop:disable Layout/LineLength\n#   long_line = true\n#   # rubocop:enable Layout/LineLength\nx = 1\n";
        let dr = disabled_ranges(src);
        assert!(
            !dr.is_disabled("Layout/LineLength", 5),
            "YARD nested comment should not create a real disable range"
        );
    }

    #[test]
    fn directive_after_other_comment_text() {
        // `# :nodoc: # rubocop:disable Foo/Bar` — the directive should be recognized
        let src = "def foo # :nodoc: # rubocop:disable Naming/PredicateMethod\n  true\nend\n";
        let dr = disabled_ranges(src);
        assert!(
            dr.is_disabled("Naming/PredicateMethod", 1),
            "directive after :nodoc: should be recognized"
        );
    }

    #[test]
    fn directive_after_steep_ignore() {
        // `# steep:ignore # rubocop:disable Metrics/BlockLength`
        let src = "Obj = Lib.build do |c| # steep:ignore # rubocop:disable Metrics/BlockLength\n  x = 1\nend\n";
        let dr = disabled_ranges(src);
        assert!(
            dr.is_disabled("Metrics/BlockLength", 1),
            "directive after steep:ignore should be recognized"
        );
    }

    #[test]
    fn directive_after_descriptive_comment() {
        // `# strip leading dot # rubocop:disable Performance/Foo`
        let src = "x = key[1..] # strip leading dot # rubocop:disable Performance/ArraySemiInfiniteRangeSlice\ny = 2\n";
        let dr = disabled_ranges(src);
        assert!(
            dr.is_disabled("Performance/ArraySemiInfiniteRangeSlice", 1),
            "directive after descriptive comment should be recognized"
        );
    }

    #[test]
    fn inline_double_hash_directive() {
        // `rescue Exception # # rubocop:disable Lint/RescueException`
        // The double-# pattern is legitimate when inline (code before the comment).
        // This must NOT be rejected as a YARD doc nested comment.
        let src = "begin\n  do_something\nrescue Exception # # rubocop:disable Lint/RescueException\n  handle_error\nend\n";
        let dr = disabled_ranges(src);
        assert!(
            dr.is_disabled("Lint/RescueException", 3),
            "inline double-# directive should be recognized, not rejected as YARD doc"
        );
    }

    #[test]
    fn double_colon_separator_treated_as_department_disable() {
        // RuboCop's COP_NAME_PATTERN = `([A-Za-z]\w+/)*(?:[A-Za-z]\w+)` splits
        // on `/` only — `:` is not `\w`. So `Department::CopName` captures only
        // `Department`, and RuboCop treats it as a department-level disable.
        // Both block and inline forms should work.
        let mut dr = disabled_ranges(
            "# rubocop:disable Rails::SkipsModelValidations\nfoo.update_attribute(:x, y)\n# rubocop:enable Rails::SkipsModelValidations\n",
        );
        assert!(
            dr.check_and_mark_used("Rails/SkipsModelValidations", 2),
            "Rails::SkipsModelValidations should suppress Rails/SkipsModelValidations via department"
        );

        // Inline form
        let mut dr2 = disabled_ranges(
            "foo.update_attribute(:x, y) # rubocop:disable Rails::SkipsModelValidations\n",
        );
        assert!(
            dr2.check_and_mark_used("Rails/SkipsModelValidations", 1),
            "inline Rails::SkipsModelValidations should suppress Rails/SkipsModelValidations via department"
        );
    }

    #[test]
    fn double_colon_old_cop_name_suppresses_via_department() {
        // `Naming::PredicateName` (old cop name with `::` separator) should
        // suppress Naming/PredicatePrefix because `:` is not part of \w in
        // RuboCop's regex, so only `Naming` is captured → department disable.
        let mut dr = disabled_ranges(
            "def has_tag?(s) # rubocop:disable Naming::PredicateName\n  true\nend\n",
        );
        assert!(
            dr.check_and_mark_used("Naming/PredicatePrefix", 1),
            "Naming::PredicateName should suppress Naming/PredicatePrefix via department"
        );
    }

    #[test]
    fn wildcard_department_disable() {
        // `# rubocop:disable Style/*` should disable all Style cops
        let src = "# rubocop:disable Style/*\nx = 1\n# rubocop:enable Style/*\ny = 2\n";
        let dr = disabled_ranges(src);
        assert!(
            dr.is_disabled("Style/IfInsideElse", 2),
            "Style/* should disable Style/IfInsideElse"
        );
        assert!(
            dr.is_disabled("Style/MissingElse", 2),
            "Style/* should disable Style/MissingElse"
        );
        assert!(
            !dr.is_disabled("Lint/Void", 2),
            "Style/* should NOT disable Lint/Void"
        );
        assert!(
            !dr.is_disabled("Style/IfInsideElse", 4),
            "Style/* should be re-enabled after enable directive"
        );
    }

    #[test]
    fn wildcard_department_disable_inline() {
        // Inline wildcard department disable
        let src = "x = 1 # rubocop:disable Style/*\ny = 2\n";
        let dr = disabled_ranges(src);
        assert!(
            dr.is_disabled("Style/IfInsideElse", 1),
            "inline Style/* should disable Style/IfInsideElse on same line"
        );
        assert!(
            !dr.is_disabled("Style/IfInsideElse", 2),
            "inline Style/* should NOT disable on next line"
        );
    }

    mod prop_tests {
        use super::*;
        use proptest::prelude::*;

        /// Build a DisabledRanges synthetically from a map of cop name -> ranges.
        fn synthetic_ranges(map: HashMap<String, Vec<(usize, usize)>>) -> DisabledRanges {
            let empty = map.is_empty();
            DisabledRanges {
                ranges: map,
                empty,
                directives: Vec::new(),
            }
        }

        /// Strategy for cop names like "Dept/CopName".
        fn cop_name_strategy() -> impl Strategy<Value = String> {
            let depts = prop::sample::select(vec![
                "Layout", "Style", "Lint", "Metrics", "Naming", "Rails", "RSpec",
            ]);
            let cops = prop::sample::select(vec![
                "Foo",
                "Bar",
                "Baz",
                "LineLength",
                "MethodLength",
                "AbcSize",
            ]);
            (depts, cops).prop_map(|(d, c)| format!("{d}/{c}"))
        }

        /// Strategy for non-overlapping sorted ranges (1-indexed lines).
        fn line_ranges_strategy() -> impl Strategy<Value = Vec<(usize, usize)>> {
            prop::collection::vec((1usize..200, 1usize..50), 0..8).prop_map(|pairs| {
                let mut ranges: Vec<(usize, usize)> = pairs
                    .into_iter()
                    .map(|(start, len)| (start, start + len))
                    .collect();
                ranges.sort_unstable();
                ranges
            })
        }

        proptest! {
            #[test]
            fn in_range_lines_are_disabled(
                cop in cop_name_strategy(),
                ranges in line_ranges_strategy(),
            ) {
                let map = HashMap::from([(cop.clone(), ranges.clone())]);
                let dr = synthetic_ranges(map);
                for &(start, end) in &ranges {
                    for line in start..=end.min(start + 5) {
                        prop_assert!(dr.is_disabled(&cop, line),
                            "{} should be disabled at line {} (range {}-{})", cop, line, start, end);
                    }
                }
            }

            #[test]
            fn out_of_range_lines_are_not_disabled(
                cop in cop_name_strategy(),
                ranges in line_ranges_strategy(),
            ) {
                let map = HashMap::from([(cop.clone(), ranges.clone())]);
                let dr = synthetic_ranges(map);
                // Test lines that should NOT be disabled (gaps between ranges)
                for line in 1usize..300 {
                    let in_any_range = ranges.iter().any(|&(s, e)| line >= s && line <= e);
                    if !in_any_range {
                        prop_assert!(!dr.check_ranges(&cop, line),
                            "{} should NOT be disabled at line {} (exact key)", cop, line);
                    }
                }
            }

            #[test]
            fn department_fallback(
                dept in prop::sample::select(vec!["Layout", "Style", "Lint", "Metrics"]),
                cop_suffix in prop::sample::select(vec!["Foo", "Bar", "Baz"]),
                ranges in line_ranges_strategy(),
            ) {
                // Disable a department, verify cop in that department is disabled
                let cop_name = format!("{dept}/{cop_suffix}");
                let map = HashMap::from([(dept.to_string(), ranges.clone())]);
                let dr = synthetic_ranges(map);
                for &(start, end) in &ranges {
                    for line in start..=end.min(start + 5) {
                        prop_assert!(dr.is_disabled(&cop_name, line),
                            "{} should be disabled via department {} at line {}",
                            cop_name, dept, line);
                    }
                }
            }

            #[test]
            fn all_disables_everything(
                cop in cop_name_strategy(),
                ranges in line_ranges_strategy(),
            ) {
                let map = HashMap::from([("all".to_string(), ranges.clone())]);
                let dr = synthetic_ranges(map);
                for &(start, end) in &ranges {
                    for line in start..=end.min(start + 5) {
                        prop_assert!(dr.is_disabled(&cop, line),
                            "{} should be disabled via 'all' at line {}", cop, line);
                    }
                }
            }

            #[test]
            fn unrelated_cop_not_disabled(
                ranges in line_ranges_strategy(),
            ) {
                // Disable "Foo/Bar", verify "Baz/Qux" is not disabled
                let map = HashMap::from([("Foo/Bar".to_string(), ranges.clone())]);
                let dr = synthetic_ranges(map);
                for &(start, end) in &ranges {
                    for line in start..=end.min(start + 5) {
                        prop_assert!(!dr.is_disabled("Baz/Qux", line),
                            "Baz/Qux should NOT be disabled when only Foo/Bar is");
                    }
                }
            }

            #[test]
            fn empty_ranges_never_disabled(cop in cop_name_strategy(), line in 1usize..1000) {
                let dr = synthetic_ranges(HashMap::new());
                prop_assert!(!dr.is_disabled(&cop, line));
                prop_assert!(dr.is_empty());
            }

            #[test]
            fn is_disabled_is_deterministic(
                cop in cop_name_strategy(),
                ranges in line_ranges_strategy(),
                line in 1usize..300,
            ) {
                let map = HashMap::from([(cop.clone(), ranges)]);
                let dr = synthetic_ranges(map);
                let first = dr.is_disabled(&cop, line);
                let second = dr.is_disabled(&cop, line);
                prop_assert_eq!(first, second, "is_disabled not deterministic");
            }
        }
    }
}
