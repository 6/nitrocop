# nitrocop-config: EnforcedStyle: always_braces

# Nested block inside lambda argument on a non-parenthesized call — ignored
register_placeholder :path, ->(resource) do
  {
    raw_value: resource.relative_path_basename_without_prefix.tap do |path|
      path.delete_prefix! "x/"
      path
    end,
  }
end

# Nested block inside interpolation within ignored lambda argument — ignored
render html->{ <<~HTML
  <ul>
  #{html_map 3.times do |i| <<~HTML
    <li>#{text->{ i }}</li>
  HTML
  end}
  </ul>
HTML
}
