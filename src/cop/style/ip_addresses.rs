use ruby_prism::Visit;

use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::codemap::CodeMap;
use crate::parse::source::SourceFile;

/// Checks for hardcoded IP addresses in string literals.
///
/// ## Investigation findings (2026-03-15, updated 2026-03-19)
///
/// Root causes of 217 FPs and 66 FNs:
///
/// **FP causes (fixed):**
/// 1. String segments inside interpolated strings (`"#{x}10.0.0.1#{y}"`) lack
///    opening delimiters. RuboCop's `StringHelp` checks `node.loc?(:begin)` and
///    skips these; we now check `opening_loc().is_some()`.
/// 2. IPv4 validation accepted octets with leading zeros (e.g., `01.02.03.04`).
///    Ruby's `Resolv::IPv4::Regex` rejects leading zeros; we now match that behavior
///    with a strict octet validator.
/// 3. Used `unescaped()` instead of `content_loc().as_slice()`, so escape sequences
///    like `"\x31.2.3.4"` would expand to `1.2.3.4` and false-positive. RuboCop
///    checks the raw source between quotes (`node.source[1...-1]`).
/// 4. IPv6 compressed validator accepted `:::` (three colons), `:::A`, and `::A:`
///    as valid. `:::` is never valid IPv6 — added early rejection. `::A:` has a
///    trailing colon creating empty groups; fixed by requiring all groups in the
///    left/right halves of `::` split to be valid hex (no empty groups allowed).
///    These patterns appear in Ruby code as scope resolution operators (`:::`) and
///    IRB completion candidates.
/// 5. String literals inside regexp interpolation (e.g., `/#{method('::1')}/`)
///    were flagged. RuboCop's `StringHelp` mixin calls `ignore_node` on regexp
///    nodes and `part_of_ignored_node?` skips all descendant strings. Switched
///    from `check_node` to `check_source` with a custom visitor that tracks
///    regexp nesting depth to replicate this behavior.
///
/// **FN causes (fixed):**
/// 1. Missing IPv4-mapped IPv6 support (`::ffff:192.168.1.1`). Ruby's
///    `Resolv::IPv6::Regex` includes `Regex_6Hex4Dec` and `Regex_CompressedHex4Dec`
///    patterns for this format.
/// 2. Missing link-local IPv6 with zone IDs (`fe80::1%lo0`). Ruby's
///    `Resolv::IPv6::Regex` includes `Regex_LinkLocal_6Hex7` and
///    `Regex_LinkLocal_CompressedHex7` patterns for `fe80` prefix addresses
///    with `%zone_id` suffixes. Zone ID allows `[-0-9A-Za-z._~]+`.
///
/// ## Investigation findings (2026-03-27)
///
/// **FP causes (fixed):**
/// 6. Percent-literal strings (`%q{10.33.33.33}`, `%(0.0.0.0)`) were flagged.
///    RuboCop's `node.source[1...-1]` strips only the first and last character,
///    yielding garbled content like `q{10.33.33.33` for percent literals, which
///    fails the `starts_with_hex_or_colon?` pre-filter. We now skip strings
///    whose opening delimiter starts with `%`.
///
/// **FN causes (fixed):**
/// 3. Compressed IPv6 with more than 7 explicit groups (`1::3:4:5:6:7:8:9`)
///    was rejected. Ruby's `Resolv::IPv6::Regex_CompressedHex` has no upper
///    bound on total group count — removed the `left + right > 7` check.
pub struct IpAddresses;

/// Maximum length of an IPv6 address string.
/// Link-local with zone ID can exceed 45 chars (e.g., fe80::1%long_interface_name).
/// Use a generous limit.
const IPV6_MAX_SIZE: usize = 80;

/// Valid zone ID character per RFC 6874: [-0-9A-Za-z._~]
fn is_valid_zone_id_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-' || b == b'.' || b == b'_' || b == b'~'
}

impl IpAddresses {
    /// Validate an IPv4 octet matching Ruby's `Resolv::IPv4::Regex256` pattern.
    /// Rejects leading zeros (e.g., "01", "001") to match Ruby's behavior.
    fn is_valid_ipv4_octet(s: &str) -> bool {
        if s.is_empty() {
            return false;
        }
        // Reject leading zeros: "0" is ok, "00", "01", "001" etc. are not
        if s.len() > 1 && s.starts_with('0') {
            return false;
        }
        matches!(s.parse::<u16>(), Ok(n) if n <= 255)
    }

    fn is_ipv4(s: &str) -> bool {
        let parts: Vec<&str> = s.split('.').collect();
        if parts.len() != 4 {
            return false;
        }
        parts.iter().all(|part| Self::is_valid_ipv4_octet(part))
    }

    /// Check if a hex group (colon-separated) is valid for IPv6.
    fn is_valid_hex_group(group: &str) -> bool {
        !group.is_empty() && group.len() <= 4 && group.bytes().all(|b| b.is_ascii_hexdigit())
    }

    fn is_ipv6(s: &str) -> bool {
        // Must not be too long
        if s.len() > IPV6_MAX_SIZE {
            return false;
        }

        // Must contain at least one colon
        if !s.contains(':') {
            return false;
        }

        // Three or more consecutive colons is never valid IPv6
        if s.contains(":::") {
            return false;
        }

        // Check for link-local with zone ID (fe80 prefix + %zone suffix)
        if let Some(pct_pos) = s.find('%') {
            return Self::is_ipv6_link_local_with_zone(s, pct_pos);
        }

        // Try IPv4-mapped IPv6 formats first (e.g., ::ffff:192.168.1.1)
        if s.contains('.') {
            return Self::is_ipv6_with_ipv4(s);
        }

        // Must only contain hex digits and colons
        if !s.bytes().all(|b| b.is_ascii_hexdigit() || b == b':') {
            return false;
        }

        // Check for :: (collapsed zeros)
        if s.contains("::") {
            return Self::is_ipv6_compressed(s);
        }

        // Full form: 8 groups of hex
        let groups: Vec<&str> = s.split(':').collect();
        if groups.len() != 8 {
            return false;
        }
        groups.iter().all(|g| Self::is_valid_hex_group(g))
    }

    /// Validate compressed IPv6 (contains ::)
    fn is_ipv6_compressed(s: &str) -> bool {
        // Can have at most one ::
        if s.matches("::").count() > 1 {
            return false;
        }
        let parts: Vec<&str> = s.split("::").collect();
        if parts.len() != 2 {
            return false;
        }
        // Validate left side: if non-empty, split by ':' and all groups must be valid hex
        // (no empty groups allowed — that would indicate a leading/trailing/extra colon)
        if !parts[0].is_empty() {
            for g in parts[0].split(':') {
                if !Self::is_valid_hex_group(g) {
                    return false;
                }
            }
        }
        // Validate right side similarly
        if !parts[1].is_empty() {
            for g in parts[1].split(':') {
                if !Self::is_valid_hex_group(g) {
                    return false;
                }
            }
        }
        // Note: Ruby's Resolv::IPv6::Regex_CompressedHex does NOT enforce a
        // maximum group count. It accepts strings like "1::3:4:5:6:7:8:9"
        // (8 explicit groups + ::) even though that exceeds 8 total groups.
        // We match this permissive behavior for RuboCop compatibility.
        true
    }

    /// Validate IPv6 addresses with embedded IPv4 (e.g., ::ffff:192.168.1.1,
    /// 64:ff9b::192.0.2.33). Matches Ruby's Resolv::IPv6::Regex_6Hex4Dec and
    /// Regex_CompressedHex4Dec patterns.
    fn is_ipv6_with_ipv4(s: &str) -> bool {
        // Find the IPv4 part: everything after the last colon.
        // The IPv4 address is always at the end after a colon separator.
        let last_colon = match s.rfind(':') {
            Some(pos) => pos,
            None => return false,
        };
        // The prefix includes the trailing colon for easier parsing
        let ipv6_prefix = &s[..=last_colon];
        let ipv4_suffix = &s[last_colon + 1..];

        // The IPv4 suffix must be a valid IPv4 address
        if !Self::is_ipv4(ipv4_suffix) {
            return false;
        }

        // The prefix must only contain hex digits and colons
        if !ipv6_prefix
            .bytes()
            .all(|b| b.is_ascii_hexdigit() || b == b':')
        {
            return false;
        }

        if ipv6_prefix.contains("::") {
            // Compressed form with IPv4 (Regex_CompressedHex4Dec)
            if ipv6_prefix.matches("::").count() > 1 {
                return false;
            }
            let parts: Vec<&str> = ipv6_prefix.split("::").collect();
            if parts.len() != 2 {
                return false;
            }
            let left_groups: Vec<&str> = if parts[0].is_empty() {
                vec![]
            } else {
                let groups: Vec<&str> = parts[0].split(':').collect();
                for g in &groups {
                    if !Self::is_valid_hex_group(g) {
                        return false;
                    }
                }
                groups
            };
            let right_part = parts[1].trim_end_matches(':');
            let right_groups: Vec<&str> = if right_part.is_empty() {
                vec![]
            } else {
                let groups: Vec<&str> = right_part.split(':').collect();
                for g in &groups {
                    if !Self::is_valid_hex_group(g) {
                        return false;
                    }
                }
                groups
            };
            // IPv4 counts as 2 groups, so hex groups + 2 <= 8
            if left_groups.len() + right_groups.len() > 5 {
                return false;
            }
            true
        } else {
            // Full form: exactly 6 hex groups (Regex_6Hex4Dec)
            // The prefix has a trailing ":" so strip it before splitting.
            let prefix_trimmed = ipv6_prefix.trim_end_matches(':');
            let groups: Vec<&str> = prefix_trimmed.split(':').collect();
            if groups.len() != 6 {
                return false;
            }
            groups.iter().all(|g| Self::is_valid_hex_group(g))
        }
    }

    /// Validate link-local IPv6 addresses with zone IDs.
    /// Matches Ruby's Resolv::IPv6::Regex_LinkLocal_6Hex7 and
    /// Regex_LinkLocal_CompressedHex7. These require:
    /// - `fe80` prefix (case-insensitive)
    /// - Valid IPv6 address body
    /// - `%` followed by one or more zone ID characters `[-0-9A-Za-z._~]`
    fn is_ipv6_link_local_with_zone(s: &str, pct_pos: usize) -> bool {
        let addr_part = &s[..pct_pos];
        let zone_part = &s[pct_pos + 1..];

        // Zone ID must be non-empty and contain only valid characters
        if zone_part.is_empty() || !zone_part.bytes().all(is_valid_zone_id_char) {
            return false;
        }

        // Must start with fe80 (case-insensitive)
        if !addr_part
            .get(..4)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("fe80"))
        {
            return false;
        }

        // The address part (without zone) must be a valid IPv6 address
        // Must only contain hex digits and colons in address part
        if !addr_part
            .bytes()
            .all(|b| b.is_ascii_hexdigit() || b == b':')
        {
            return false;
        }

        // After "fe80", must have a colon
        if addr_part.len() < 5 || addr_part.as_bytes()[4] != b':' {
            return false;
        }

        // Validate as a normal IPv6 address (reuse existing validation)
        if addr_part.contains("::") {
            Self::is_ipv6_compressed(addr_part)
        } else {
            let groups: Vec<&str> = addr_part.split(':').collect();
            groups.len() == 8 && groups.iter().all(|g| Self::is_valid_hex_group(g))
        }
    }

    /// Pre-filter: the first character must be a hex digit or colon.
    /// Matches RuboCop's `starts_with_hex_or_colon?` optimization.
    fn starts_with_hex_or_colon(s: &str) -> bool {
        match s.as_bytes().first() {
            Some(b) => b.is_ascii_hexdigit() || *b == b':',
            None => false,
        }
    }

    /// Check a single string node for IP address content.
    fn check_string_node(
        &self,
        source: &SourceFile,
        string_node: &ruby_prism::StringNode<'_>,
        config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        // Skip string segments inside interpolated strings (no opening delimiter).
        // Matches RuboCop's StringHelp `node.loc?(:begin)` check.
        let opening_loc = match string_node.opening_loc() {
            Some(loc) => loc,
            None => return,
        };

        // Skip percent-literal strings (%q{...}, %Q{...}, %(...), etc.).
        // RuboCop uses `node.source[1...-1]` which strips only the first and last
        // character. For percent literals like `%q{10.33.33.33}`, this yields
        // `q{10.33.33.33` which fails the `starts_with_hex_or_colon?` pre-filter.
        if opening_loc.as_slice().first() == Some(&b'%') {
            return;
        }

        // Use raw source content between quotes (not unescaped) to match
        // RuboCop's `node.source[1...-1]` behavior. This avoids FPs from
        // escape sequences like "\x31.2.3.4" expanding to "1.2.3.4".
        let content_loc = string_node.content_loc();
        let content_bytes = content_loc.as_slice();
        let content_str = match std::str::from_utf8(content_bytes) {
            Ok(s) => s,
            Err(_) => return,
        };

        if content_str.is_empty() {
            return;
        }

        // Quick pre-filter: string too long or doesn't start with hex/colon
        if content_str.len() > IPV6_MAX_SIZE || !Self::starts_with_hex_or_colon(content_str) {
            return;
        }

        let allowed = config
            .get_string_array("AllowedAddresses")
            .or_else(|| Some(vec!["::".to_string()]));

        // For allowed address comparison, strip zone ID if present
        let content_for_allowed = if let Some(pct_pos) = content_str.find('%') {
            &content_str[..pct_pos]
        } else {
            content_str
        };

        // Check if it's in allowed addresses (case-insensitive)
        if let Some(ref allowed_list) = allowed {
            let content_lower = content_for_allowed.to_lowercase();
            for addr in allowed_list {
                if addr.to_lowercase() == content_lower {
                    return;
                }
            }
        }

        let is_ip = Self::is_ipv4(content_str) || Self::is_ipv6(content_str);

        if is_ip {
            let loc = string_node.location();
            let (line, column) = source.offset_to_line_col(loc.start_offset());
            diagnostics.push(self.diagnostic(
                source,
                line,
                column,
                "Do not hardcode IP addresses.".to_string(),
            ));
        }
    }
}

impl Cop for IpAddresses {
    fn name(&self) -> &'static str {
        "Style/IpAddresses"
    }

    fn default_enabled(&self) -> bool {
        false
    }

    fn default_exclude(&self) -> &'static [&'static str] {
        &["**/*.gemfile", "**/Gemfile", "**/gems.rb", "**/*.gemspec"]
    }

    fn check_source(
        &self,
        source: &SourceFile,
        parse_result: &ruby_prism::ParseResult<'_>,
        _code_map: &CodeMap,
        config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        _corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        let mut visitor = IpAddressVisitor {
            cop: self,
            source,
            config,
            in_regexp_depth: 0,
            diagnostics: Vec::new(),
        };
        visitor.visit(&parse_result.node());
        diagnostics.extend(visitor.diagnostics);
    }
}

/// AST visitor that tracks regexp nesting to skip string nodes inside regexps.
/// Matches RuboCop's `StringHelp#on_regexp` which calls `ignore_node(node)`,
/// causing all descendant `str` nodes to be skipped via `part_of_ignored_node?`.
struct IpAddressVisitor<'a, 'src> {
    cop: &'a IpAddresses,
    source: &'src SourceFile,
    config: &'a CopConfig,
    in_regexp_depth: u32,
    diagnostics: Vec<Diagnostic>,
}

impl<'pr> Visit<'pr> for IpAddressVisitor<'_, '_> {
    fn visit_string_node(&mut self, node: &ruby_prism::StringNode<'pr>) {
        if self.in_regexp_depth == 0 {
            self.cop
                .check_string_node(self.source, node, self.config, &mut self.diagnostics);
        }
        // StringNode is a leaf, no children to visit
    }

    fn visit_interpolated_regular_expression_node(
        &mut self,
        node: &ruby_prism::InterpolatedRegularExpressionNode<'pr>,
    ) {
        self.in_regexp_depth += 1;
        ruby_prism::visit_interpolated_regular_expression_node(self, node);
        self.in_regexp_depth -= 1;
    }

    fn visit_interpolated_match_last_line_node(
        &mut self,
        node: &ruby_prism::InterpolatedMatchLastLineNode<'pr>,
    ) {
        self.in_regexp_depth += 1;
        ruby_prism::visit_interpolated_match_last_line_node(self, node);
        self.in_regexp_depth -= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(IpAddresses, "cops/style/ip_addresses");

    #[test]
    fn test_ipv4_validation() {
        assert!(IpAddresses::is_ipv4("127.0.0.1"));
        assert!(IpAddresses::is_ipv4("0.0.0.0"));
        assert!(IpAddresses::is_ipv4("255.255.255.255"));
        assert!(IpAddresses::is_ipv4("192.168.1.1"));

        // Leading zeros rejected (matching Ruby's Resolv::IPv4::Regex)
        assert!(!IpAddresses::is_ipv4("01.02.03.04"));
        assert!(!IpAddresses::is_ipv4("001.002.003.004"));
        assert!(!IpAddresses::is_ipv4("1.2.3.04"));
        assert!(!IpAddresses::is_ipv4("192.168.001.001"));
        assert!(!IpAddresses::is_ipv4("10.0.0.01"));

        // Invalid
        assert!(!IpAddresses::is_ipv4("578.194.591.059"));
        assert!(!IpAddresses::is_ipv4("1.2.3"));
        assert!(!IpAddresses::is_ipv4("1.2.3.4.5"));
        assert!(!IpAddresses::is_ipv4(""));
    }

    #[test]
    fn test_ipv6_validation() {
        // Full form
        assert!(IpAddresses::is_ipv6(
            "2001:0db8:85a3:0000:0000:8a2e:0370:7334"
        ));
        assert!(IpAddresses::is_ipv6("0:0:0:0:0:0:0:0"));
        assert!(IpAddresses::is_ipv6("0:0:0:0:0:0:0:1"));

        // Compressed
        assert!(IpAddresses::is_ipv6("2001:db8::1"));
        assert!(IpAddresses::is_ipv6("::1"));
        assert!(IpAddresses::is_ipv6("1::"));
        assert!(IpAddresses::is_ipv6("::"));
        assert!(IpAddresses::is_ipv6("2001:db8:85a3::8a2e:370:7334"));
        assert!(IpAddresses::is_ipv6("::ffff:0:0"));

        // IPv4-mapped
        assert!(IpAddresses::is_ipv6("::ffff:192.168.1.1"));
        assert!(IpAddresses::is_ipv6("64:ff9b::192.0.2.33"));

        // Link-local with zone ID
        assert!(IpAddresses::is_ipv6("fe80::1%lo0"));
        assert!(IpAddresses::is_ipv6("fe80::200:11ff:fe22:1122%5"));
        assert!(IpAddresses::is_ipv6("fe80:0:0:0:0:0:0:1%eth0"));
        assert!(IpAddresses::is_ipv6("FE80::1%lo0")); // case-insensitive

        // Invalid
        assert!(!IpAddresses::is_ipv6("2001:db8::1xyz"));
        assert!(!IpAddresses::is_ipv6(""));
        assert!(!IpAddresses::is_ipv6("not-an-ip"));

        // Triple colons and related patterns are NOT valid IPv6
        assert!(!IpAddresses::is_ipv6(":::"));
        assert!(!IpAddresses::is_ipv6(":::A"));
        assert!(!IpAddresses::is_ipv6("::A:"));
        assert!(!IpAddresses::is_ipv6(":::A:"));

        // Zone ID without fe80 prefix is not valid
        assert!(!IpAddresses::is_ipv6("dead::beef%eth0"));
        // Empty zone ID is not valid
        assert!(!IpAddresses::is_ipv6("fe80::1%"));
    }

    #[test]
    fn test_ipv4_octet_no_leading_zeros() {
        assert!(IpAddresses::is_valid_ipv4_octet("0"));
        assert!(IpAddresses::is_valid_ipv4_octet("1"));
        assert!(IpAddresses::is_valid_ipv4_octet("255"));
        assert!(!IpAddresses::is_valid_ipv4_octet("00"));
        assert!(!IpAddresses::is_valid_ipv4_octet("01"));
        assert!(!IpAddresses::is_valid_ipv4_octet("001"));
        assert!(!IpAddresses::is_valid_ipv4_octet("256"));
        assert!(!IpAddresses::is_valid_ipv4_octet(""));
    }
}
