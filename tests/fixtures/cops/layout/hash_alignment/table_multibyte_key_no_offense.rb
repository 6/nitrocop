# nitrocop-config: EnforcedStyle: table, table, ignore_implicit
# Regression: byte-width vs char-width key length used to cause FPs when keys
# contained multi-byte characters (emoji, CJK). The hashes below are aligned
# correctly in both the char-based column sense and the RuboCop sense.
EMOJI = {
  "🚀" => "rocket",
  "💪" => "muscle"
}

JAPAN = {
  "東京都" => "Tokyo",
  "大阪府" => "Osaka",
  "京都府" => "Kyoto"
}

SINGLE = {
  "🚀" => "rocket"
}
